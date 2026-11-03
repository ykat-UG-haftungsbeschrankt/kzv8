(function (global) {
  "use strict";

  const SECTOR_SIZE = 512;
  const MAX_FILE_SIZE = 4 * 1024 * 1024;
  const PARTITION_TYPES = new Set([0x01, 0x04, 0x06, 0x0b, 0x0c, 0x0e]);
  const block = global.kz.block;
  const sectorCache = new Map();
  let volume;

  function fail(message) {
    throw new Error(`VFAT: ${message}`);
  }

  function isBytes(value) {
    return value !== null && value !== undefined &&
      Object.prototype.toString.call(value) === "[object Uint8Array]";
  }

  function read16(bytes, offset) {
    return bytes[offset] | (bytes[offset + 1] << 8);
  }

  function read32(bytes, offset) {
    return (bytes[offset] | (bytes[offset + 1] << 8) |
      (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0;
  }

  function write32(bytes, offset, value) {
    bytes[offset] = value & 0xff;
    bytes[offset + 1] = (value >>> 8) & 0xff;
    bytes[offset + 2] = (value >>> 16) & 0xff;
    bytes[offset + 3] = (value >>> 24) & 0xff;
  }

  function readSector(sector) {
    const key = sector.toString();
    const cached = sectorCache.get(key);
    if (cached) return cached.slice();
    if (sector < 0n || sector >= block.capacitySectors()) {
      fail("sector is outside the block device");
    }
    const bytes = block.read(sector, SECTOR_SIZE);
    if (!isBytes(bytes) || bytes.length !== SECTOR_SIZE) {
      fail("block read returned an invalid sector");
    }
    const copy = new Uint8Array(bytes);
    sectorCache.set(key, copy);
    return copy.slice();
  }

  function writeSector(sector, bytes) {
    if (!isBytes(bytes) || bytes.length !== SECTOR_SIZE) {
      fail("block writes must contain one sector");
    }
    const copy = new Uint8Array(bytes);
    if (block.write(sector, copy) !== SECTOR_SIZE) {
      fail("block write did not complete");
    }
    sectorCache.set(sector.toString(), copy);
  }

  function mount() {
    if (volume) return volume;
    const capacity = block.capacitySectors();
    if (typeof capacity !== "bigint" || capacity < 2n) {
      fail("block device is too small");
    }

    const firstSector = readSector(0n);
    let partitionStart = 0n;
    let partitionSectors = capacity;
    if (firstSector[510] === 0x55 && firstSector[511] === 0xaa) {
      for (let index = 0; index < 4; index++) {
        const offset = 446 + index * 16;
        const type = firstSector[offset + 4];
        const start = BigInt(read32(firstSector, offset + 8));
        const count = BigInt(read32(firstSector, offset + 12));
        if (PARTITION_TYPES.has(type) && start > 0n && count > 0n &&
            start + count <= capacity) {
          partitionStart = start;
          partitionSectors = count;
          break;
        }
      }
    }

    const boot = partitionStart === 0n ? firstSector : readSector(partitionStart);
    if (boot[510] !== 0x55 || boot[511] !== 0xaa) {
      fail("missing FAT boot signature");
    }
    const bytesPerSector = read16(boot, 11);
    const sectorsPerCluster = boot[13];
    const reservedSectors = read16(boot, 14);
    const fatCount = boot[16];
    const rootEntryCount = read16(boot, 17);
    const totalSectors = read16(boot, 19) || read32(boot, 32);
    const fat16Sectors = read16(boot, 22);
    const fat32Sectors = read32(boot, 36);
    if (bytesPerSector !== SECTOR_SIZE || sectorsPerCluster === 0 ||
        (sectorsPerCluster & (sectorsPerCluster - 1)) !== 0 ||
        reservedSectors === 0 || fatCount === 0 || fatCount > 2 ||
        totalSectors === 0 || BigInt(totalSectors) > partitionSectors) {
      fail("invalid FAT BPB");
    }

    const fatSectors = fat16Sectors || fat32Sectors;
    const rootDirectorySectors = Math.ceil(rootEntryCount * 32 / SECTOR_SIZE);
    const dataSectors = totalSectors - reservedSectors -
      fatCount * fatSectors - rootDirectorySectors;
    const clusterCount = Math.floor(dataSectors / sectorsPerCluster);
    const type = clusterCount < 4085 ? 12 : clusterCount < 65525 ? 16 : 32;
    if (dataSectors <= 0 || type === 12 || fatSectors === 0 ||
        (type === 16 && (fat16Sectors === 0 || rootEntryCount === 0)) ||
        (type === 32 && (fat16Sectors !== 0 || rootEntryCount !== 0))) {
      fail("only FAT16 and FAT32 volumes are supported");
    }

    const firstFatSector = partitionStart + BigInt(reservedSectors);
    const firstRootSector = firstFatSector + BigInt(fatCount * fatSectors);
    const firstDataSector = firstRootSector + BigInt(rootDirectorySectors);
    const rootCluster = type === 32 ? read32(boot, 44) & 0x0fffffff : 0;
    if (type === 32 && (rootCluster < 2 || rootCluster >= clusterCount + 2)) {
      fail("invalid FAT32 root cluster");
    }

    volume = {
      type,
      sectorsPerCluster,
      clusterCount,
      firstFatSector,
      fatSectors,
      fatCount,
      rootDirectorySectors,
      firstRootSector,
      firstDataSector,
      rootCluster,
    };
    return volume;
  }

  function fatEntry(cluster) {
    const fs = mount();
    const width = fs.type === 16 ? 2 : 4;
    const byteOffset = cluster * width;
    const sector = fs.firstFatSector + BigInt(Math.floor(byteOffset / SECTOR_SIZE));
    const offset = byteOffset % SECTOR_SIZE;
    const bytes = readSector(sector);
    return fs.type === 16 ? read16(bytes, offset) : read32(bytes, offset) & 0x0fffffff;
  }

  function clusterChain(firstCluster) {
    const fs = mount();
    const chain = [];
    const seen = new Set();
    let cluster = firstCluster;
    while (cluster >= 2 && cluster < fs.clusterCount + 2) {
      if (seen.has(cluster) || chain.length >= fs.clusterCount) {
        fail("cyclic FAT cluster chain");
      }
      seen.add(cluster);
      chain.push(cluster);
      const next = fatEntry(cluster);
      if ((fs.type === 16 && next >= 0xfff8) ||
          (fs.type === 32 && next >= 0x0ffffff8)) {
        return chain;
      }
      if (next < 2 || (fs.type === 16 && next >= 0xfff0) ||
          (fs.type === 32 && next >= 0x0ffffff0)) {
        fail("invalid FAT cluster chain");
      }
      cluster = next;
    }
    if (firstCluster === 0) return [];
    fail("unterminated FAT cluster chain");
  }

  function clusterSector(cluster, index) {
    const fs = mount();
    return fs.firstDataSector +
      BigInt((cluster - 2) * fs.sectorsPerCluster + index);
  }

  function shortName(entry) {
    let base = "";
    let extension = "";
    for (let index = 0; index < 8 && entry[index] !== 0x20; index++) {
      const value = index === 0 && entry[index] === 0x05 ? 0xe5 : entry[index];
      base += String.fromCharCode(value);
    }
    for (let index = 8; index < 11 && entry[index] !== 0x20; index++) {
      extension += String.fromCharCode(entry[index]);
    }
    return extension ? `${base}.${extension}` : base;
  }

  function lfnChecksum(entry) {
    let checksum = 0;
    for (let index = 0; index < 11; index++) {
      checksum = (((checksum & 1) << 7) + (checksum >>> 1) + entry[index]) & 0xff;
    }
    return checksum;
  }

  function lfnFragment(entry) {
    const offsets = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    let fragment = "";
    for (const offset of offsets) {
      const code = read16(entry, offset);
      if (code === 0 || code === 0xffff) break;
      fragment += String.fromCharCode(code);
    }
    return fragment;
  }

  function forEachDirectoryEntry(directory, callback) {
    const fs = mount();
    let found = false;
    let ended = false;
    let lfnParts = [];
    let checksum = -1;

    function scanSector(sector) {
      const bytes = readSector(sector);
      for (let offset = 0; offset < SECTOR_SIZE; offset += 32) {
        const first = bytes[offset];
        if (first === 0) {
          ended = true;
          return;
        }
        if (first === 0xe5) {
          lfnParts = [];
          checksum = -1;
          continue;
        }
        const entry = bytes.subarray(offset, offset + 32);
        const attributes = entry[11];
        if (attributes === 0x0f) {
          const ordinal = first & 0x1f;
          if ((first & 0x40) !== 0) {
            lfnParts = [];
            checksum = entry[13];
          }
          if (ordinal > 0 && (checksum === -1 || checksum === entry[13])) {
            lfnParts[ordinal - 1] = lfnFragment(entry);
          }
          continue;
        }
        if ((attributes & 0x08) !== 0) {
          lfnParts = [];
          checksum = -1;
          continue;
        }

        let name = shortName(entry);
        if (lfnParts.length > 0 && checksum === lfnChecksum(entry) &&
            lfnParts.every((part) => typeof part === "string")) {
          name = lfnParts.join("");
        }
        lfnParts = [];
        checksum = -1;
        const clusterHigh = fs.type === 32 ? read16(entry, 20) : 0;
        const cluster = (clusterHigh << 16) | read16(entry, 26);
        const record = {
          name,
          attributes,
          cluster,
          size: read32(entry, 28),
          sector,
          offset,
        };
        if (callback(record)) {
          found = true;
          return;
        }
      }
    }

    if (directory.root && fs.type === 16) {
      for (let index = 0; index < fs.rootDirectorySectors && !ended && !found; index++) {
        scanSector(fs.firstRootSector + BigInt(index));
      }
    } else {
      const rootCluster = directory.root ? fs.rootCluster : directory.cluster;
      for (const cluster of clusterChain(rootCluster)) {
        for (let index = 0; index < fs.sectorsPerCluster && !ended && !found; index++) {
          scanSector(clusterSector(cluster, index));
        }
        if (ended || found) break;
      }
    }
    return found;
  }

  function normalizePath(path) {
    if (typeof path !== "string") fail("path must be a string");
    const parts = [];
    for (const component of path.replace(/\\/g, "/").split("/")) {
      if (!component || component === ".") continue;
      if (component === "..") {
        if (parts.length === 0) fail("path escapes the volume root");
        parts.pop();
      } else {
        parts.push(component);
      }
    }
    if (parts.length === 0 || parts[0].toLowerCase() !== "app") {
      fail("module paths must stay under /app");
    }
    return parts;
  }

  function lookup(path) {
    const parts = normalizePath(path);
    let directory = { root: true, cluster: 0 };
    let result = null;
    for (let index = 0; index < parts.length; index++) {
      const wanted = parts[index].toLowerCase();
      result = null;
      forEachDirectoryEntry(directory, (entry) => {
        if (entry.name.toLowerCase() === wanted) {
          result = entry;
          return true;
        }
        return false;
      });
      if (result === null) fail(`file not found: /${parts.join("/")}`);
      if (index + 1 < parts.length) {
        if ((result.attributes & 0x10) === 0) fail("path component is not a directory");
        directory = { root: false, cluster: result.cluster };
      }
    }
    if ((result.attributes & 0x10) !== 0) fail("path names a directory");
    return result;
  }

  function readFile(path) {
    const entry = lookup(path);
    if (entry.size > MAX_FILE_SIZE) fail("file exceeds the size limit");
    const chain = clusterChain(entry.cluster);
    const fs = mount();
    const clusterBytes = fs.sectorsPerCluster * SECTOR_SIZE;
    if (entry.size > chain.length * clusterBytes) fail("file exceeds its cluster chain");
    const output = new Uint8Array(entry.size);
    let written = 0;
    for (const cluster of chain) {
      for (let index = 0; index < fs.sectorsPerCluster && written < output.length; index++) {
        const sector = readSector(clusterSector(cluster, index));
        const amount = Math.min(SECTOR_SIZE, output.length - written);
        output.set(sector.subarray(0, amount), written);
        written += amount;
      }
      if (written === output.length) break;
    }
    return output;
  }

  function writeFile(path, contents) {
    if (!isBytes(contents)) fail("file contents must be a Uint8Array");
    if (contents.length > MAX_FILE_SIZE) fail("file exceeds the size limit");
    const entry = lookup(path);
    if ((entry.attributes & 0x01) !== 0) fail("file is read-only");
    const chain = clusterChain(entry.cluster);
    const fs = mount();
    const capacity = chain.length * fs.sectorsPerCluster * SECTOR_SIZE;
    if (contents.length > capacity) fail("writes cannot extend a cluster chain");

    let offset = 0;
    for (const cluster of chain) {
      for (let index = 0; index < fs.sectorsPerCluster && offset < contents.length; index++) {
        const sector = new Uint8Array(SECTOR_SIZE);
        const amount = Math.min(SECTOR_SIZE, contents.length - offset);
        sector.set(contents.subarray(offset, offset + amount));
        writeSector(clusterSector(cluster, index), sector);
        offset += amount;
      }
      if (offset === contents.length) break;
    }

    const directorySector = readSector(entry.sector);
    write32(directorySector, entry.offset + 28, contents.length);
    writeSector(entry.sector, directorySector);
  }

  function uriPath(uri) {
    const prefix = "virtiovfat:///";
    if (typeof uri !== "string" || !uri.startsWith(prefix)) {
      fail("expected a virtiovfat:/// URI");
    }
    return `/${uri.slice(prefix.length)}`;
  }

  function canonicalUri(specifier, referrer) {
    if (typeof specifier !== "string" || specifier.includes("?") || specifier.includes("#")) {
      fail("invalid module specifier");
    }
    let path;
    if (specifier.startsWith("virtiovfat:///")) {
      path = uriPath(specifier);
    } else if (specifier.startsWith("/")) {
      path = specifier;
    } else {
      const base = referrer ? uriPath(referrer) : "/app/main.js";
      path = `${base.slice(0, base.lastIndexOf("/") + 1)}${specifier}`;
    }
    const parts = normalizePath(path);
    return `virtiovfat:///${parts.join("/")}`.replace(/[^/]+/g, (part) => part.toLowerCase());
  }

  function readModuleSync(specifier, referrer) {
    const url = canonicalUri(specifier, referrer);
    return { url, bytes: readFile(uriPath(url)) };
  }

  async function readModule(specifier, referrer) {
    return readModuleSync(specifier, referrer);
  }

  global.kz.vfat = { readFile, writeFile };
  global.__kzVfatReadModule = readModule;
  global.__kzVfatReadModuleSync = readModuleSync;
})(globalThis);
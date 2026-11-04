interface VfatVolume {
  type: 16 | 32;
  sectorsPerCluster: number;
  clusterCount: number;
  firstFatSector: bigint;
  fatSectors: number;
  fatCount: number;
  rootDirectorySectors: number;
  firstRootSector: bigint;
  firstDataSector: bigint;
  rootCluster: number;
}

interface VfatDirectory {
  root: boolean;
  cluster: number;
}

interface VfatDirectoryEntry {
  name: string;
  attributes: number;
  cluster: number;
  size: number;
  sector: bigint;
  offset: number;
}

interface VfatModuleSource {
  url: string;
  bytes: Uint8Array;
}

type VfatGlobal = typeof globalThis & {
  kz: typeof kz;
  __kzVfatReadModule: (
    specifier: string,
    referrer: string,
  ) => Promise<VfatModuleSource>;
  __kzVfatReadModuleSync: (
    specifier: string,
    referrer: string,
  ) => VfatModuleSource;
};

class Vfat {
  private static readonly SECTOR_SIZE = 512;
  private static readonly MAX_FILE_SIZE = 4 * 1024 * 1024;
  private static readonly PARTITION_TYPES =
    new Set([0x01, 0x04, 0x06, 0x0b, 0x0c, 0x0e]);

  private readonly sectorCache = new Map<string, Uint8Array>();
  private volume: VfatVolume | undefined;

  constructor(private readonly block: KzBlockApi) {}

  private fail(message: string): never {
    throw new Error(`VFAT: ${message}`);
  }

  private isBytes(value: unknown): value is Uint8Array {
    return value !== null && value !== undefined &&
      Object.prototype.toString.call(value) === "[object Uint8Array]";
  }

  private read16(bytes: Uint8Array, offset: number): number {
    return bytes[offset] | (bytes[offset + 1] << 8);
  }

  private read32(bytes: Uint8Array, offset: number): number {
    return (bytes[offset] | (bytes[offset + 1] << 8) |
      (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0;
  }

  private write32(bytes: Uint8Array, offset: number, value: number): void {
    bytes[offset] = value & 0xff;
    bytes[offset + 1] = (value >>> 8) & 0xff;
    bytes[offset + 2] = (value >>> 16) & 0xff;
    bytes[offset + 3] = (value >>> 24) & 0xff;
  }

  private readSector(sector: bigint): Uint8Array {
    const key = sector.toString();
    const cached = this.sectorCache.get(key);
    if (cached) return cached.slice();
    if (sector < 0n || sector >= this.block.capacitySectors()) {
      this.fail("sector is outside the block device");
    }
    const bytes = this.block.read(sector, Vfat.SECTOR_SIZE);
    if (!this.isBytes(bytes) || bytes.length !== Vfat.SECTOR_SIZE) {
      this.fail("block read returned an invalid sector");
    }
    const copy = new Uint8Array(bytes);
    this.sectorCache.set(key, copy);
    return copy.slice();
  }

  private writeSector(sector: bigint, bytes: Uint8Array): void {
    if (!this.isBytes(bytes) || bytes.length !== Vfat.SECTOR_SIZE) {
      this.fail("block writes must contain one sector");
    }
    const copy = new Uint8Array(bytes);
    if (this.block.write(sector, copy) !== Vfat.SECTOR_SIZE) {
      this.fail("block write did not complete");
    }
    this.sectorCache.set(sector.toString(), copy);
  }

  private mount(): VfatVolume {
    if (this.volume) return this.volume;
    const capacity = this.block.capacitySectors();
    if (typeof capacity !== "bigint" || capacity < 2n) {
      this.fail("block device is too small");
    }

    const firstSector = this.readSector(0n);
    let partitionStart = 0n;
    let partitionSectors = capacity;
    if (firstSector[510] === 0x55 && firstSector[511] === 0xaa) {
      for (let index = 0; index < 4; index++) {
        const offset = 446 + index * 16;
        const type = firstSector[offset + 4];
        const start = BigInt(this.read32(firstSector, offset + 8));
        const count = BigInt(this.read32(firstSector, offset + 12));
        if (Vfat.PARTITION_TYPES.has(type) && start > 0n && count > 0n &&
            start + count <= capacity) {
          partitionStart = start;
          partitionSectors = count;
          break;
        }
      }
    }

    const boot = partitionStart === 0n ? firstSector : this.readSector(partitionStart);
    if (boot[510] !== 0x55 || boot[511] !== 0xaa) {
      this.fail("missing FAT boot signature");
    }
    const bytesPerSector = this.read16(boot, 11);
    const sectorsPerCluster = boot[13];
    const reservedSectors = this.read16(boot, 14);
    const fatCount = boot[16];
    const rootEntryCount = this.read16(boot, 17);
    const totalSectors = this.read16(boot, 19) || this.read32(boot, 32);
    const fat16Sectors = this.read16(boot, 22);
    const fat32Sectors = this.read32(boot, 36);
    if (bytesPerSector !== Vfat.SECTOR_SIZE || sectorsPerCluster === 0 ||
        (sectorsPerCluster & (sectorsPerCluster - 1)) !== 0 ||
        reservedSectors === 0 || fatCount === 0 || fatCount > 2 ||
        totalSectors === 0 || BigInt(totalSectors) > partitionSectors) {
      this.fail("invalid FAT BPB");
    }

    const fatSectors = fat16Sectors || fat32Sectors;
    const rootDirectorySectors = Math.ceil(rootEntryCount * 32 / Vfat.SECTOR_SIZE);
    const dataSectors = totalSectors - reservedSectors -
      fatCount * fatSectors - rootDirectorySectors;
    const clusterCount = Math.floor(dataSectors / sectorsPerCluster);
    const type: 12 | 16 | 32 =
      clusterCount < 4085 ? 12 : clusterCount < 65525 ? 16 : 32;
    if (dataSectors <= 0 || type === 12 || fatSectors === 0 ||
        (type === 16 && (fat16Sectors === 0 || rootEntryCount === 0)) ||
        (type === 32 && (fat16Sectors !== 0 || rootEntryCount !== 0))) {
      this.fail("only FAT16 and FAT32 volumes are supported");
    }

    const firstFatSector = partitionStart + BigInt(reservedSectors);
    const firstRootSector = firstFatSector + BigInt(fatCount * fatSectors);
    const firstDataSector = firstRootSector + BigInt(rootDirectorySectors);
    const rootCluster = type === 32 ? this.read32(boot, 44) & 0x0fffffff : 0;
    if (type === 32 && (rootCluster < 2 || rootCluster >= clusterCount + 2)) {
      this.fail("invalid FAT32 root cluster");
    }

    const mountedVolume: VfatVolume = {
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
    this.volume = mountedVolume;
    return mountedVolume;
  }

  private fatEntry(cluster: number): number {
    const fs = this.mount();
    const width = fs.type === 16 ? 2 : 4;
    const byteOffset = cluster * width;
    const sector = fs.firstFatSector + BigInt(Math.floor(byteOffset / Vfat.SECTOR_SIZE));
    const offset = byteOffset % Vfat.SECTOR_SIZE;
    const bytes = this.readSector(sector);
    return fs.type === 16
      ? this.read16(bytes, offset)
      : this.read32(bytes, offset) & 0x0fffffff;
  }

  private clusterChain(firstCluster: number): number[] {
    const fs = this.mount();
    const chain: number[] = [];
    const seen = new Set<number>();
    let cluster = firstCluster;
    while (cluster >= 2 && cluster < fs.clusterCount + 2) {
      if (seen.has(cluster) || chain.length >= fs.clusterCount) {
        this.fail("cyclic FAT cluster chain");
      }
      seen.add(cluster);
      chain.push(cluster);
      const next = this.fatEntry(cluster);
      if ((fs.type === 16 && next >= 0xfff8) ||
          (fs.type === 32 && next >= 0x0ffffff8)) {
        return chain;
      }
      if (next < 2 || (fs.type === 16 && next >= 0xfff0) ||
          (fs.type === 32 && next >= 0x0ffffff0)) {
        this.fail("invalid FAT cluster chain");
      }
      cluster = next;
    }
    if (firstCluster === 0) return [];
    this.fail("unterminated FAT cluster chain");
  }

  private clusterSector(cluster: number, index: number): bigint {
    const fs = this.mount();
    return fs.firstDataSector +
      BigInt((cluster - 2) * fs.sectorsPerCluster + index);
  }

  private shortName(entry: Uint8Array): string {
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

  private lfnChecksum(entry: Uint8Array): number {
    let checksum = 0;
    for (let index = 0; index < 11; index++) {
      checksum = (((checksum & 1) << 7) + (checksum >>> 1) + entry[index]) & 0xff;
    }
    return checksum;
  }

  private lfnFragment(entry: Uint8Array): string {
    const offsets = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    let fragment = "";
    for (const offset of offsets) {
      const code = this.read16(entry, offset);
      if (code === 0 || code === 0xffff) break;
      fragment += String.fromCharCode(code);
    }
    return fragment;
  }

  private findDirectoryEntry(
    directory: VfatDirectory,
    callback: (entry: VfatDirectoryEntry) => boolean,
  ): VfatDirectoryEntry | null {
    const fs = this.mount();
    let found: VfatDirectoryEntry | null = null;
    let ended = false;
    let lfnParts: string[] = [];
    let checksum = -1;

    const scanSector = (sector: bigint): void => {
      const bytes = this.readSector(sector);
      for (let offset = 0; offset < Vfat.SECTOR_SIZE; offset += 32) {
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
            lfnParts[ordinal - 1] = this.lfnFragment(entry);
          }
          continue;
        }
        if ((attributes & 0x08) !== 0) {
          lfnParts = [];
          checksum = -1;
          continue;
        }

        let name = this.shortName(entry);
        if (lfnParts.length > 0 && checksum === this.lfnChecksum(entry) &&
            lfnParts.every((part) => typeof part === "string")) {
          name = lfnParts.join("");
        }
        lfnParts = [];
        checksum = -1;
        const clusterHigh = fs.type === 32 ? this.read16(entry, 20) : 0;
        const cluster = (clusterHigh << 16) | this.read16(entry, 26);
        const record: VfatDirectoryEntry = {
          name,
          attributes,
          cluster,
          size: this.read32(entry, 28),
          sector,
          offset,
        };
        if (callback(record)) {
          found = record;
          return;
        }
      }
    };

    if (directory.root && fs.type === 16) {
      for (let index = 0; index < fs.rootDirectorySectors && !ended && found === null; index++) {
        scanSector(fs.firstRootSector + BigInt(index));
      }
    } else {
      const rootCluster = directory.root ? fs.rootCluster : directory.cluster;
      for (const cluster of this.clusterChain(rootCluster)) {
        for (let index = 0; index < fs.sectorsPerCluster && !ended && found === null; index++) {
          scanSector(this.clusterSector(cluster, index));
        }
        if (ended || found !== null) break;
      }
    }
    return found;
  }

  private normalizePath(path: string): string[] {
    const parts: string[] = [];
    for (const component of path.replace(/\\/g, "/").split("/")) {
      if (!component || component === ".") continue;
      if (component === "..") {
        if (parts.length === 0) this.fail("path escapes the volume root");
        parts.pop();
      } else {
        parts.push(component);
      }
    }
    if (parts.length === 0 || parts[0].toLowerCase() !== "app") {
      this.fail("module paths must stay under /app");
    }
    return parts;
  }

  private lookup(path: string): VfatDirectoryEntry {
    const parts = this.normalizePath(path);
    let directory: VfatDirectory = { root: true, cluster: 0 };
      let result: VfatDirectoryEntry | null = null;
    for (let index = 0; index < parts.length; index++) {
      const wanted = parts[index].toLowerCase();
        result = this.findDirectoryEntry(
        directory,
        (entry) => entry.name.toLowerCase() === wanted,
      );
      if (result === null) this.fail(`file not found: /${parts.join("/")}`);
      if (index + 1 < parts.length) {
        if ((result.attributes & 0x10) === 0) {
          this.fail("path component is not a directory");
        }
        directory = { root: false, cluster: result.cluster };
      }
    }
    if (result === null) this.fail("path names no file");
    if ((result.attributes & 0x10) !== 0) this.fail("path names a directory");
    return result;
  }

  readFile(path: string): Uint8Array {
    const entry = this.lookup(path);
    if (entry.size > Vfat.MAX_FILE_SIZE) this.fail("file exceeds the size limit");
    const chain = this.clusterChain(entry.cluster);
    const fs = this.mount();
    const clusterBytes = fs.sectorsPerCluster * Vfat.SECTOR_SIZE;
    if (entry.size > chain.length * clusterBytes) {
      this.fail("file exceeds its cluster chain");
    }
    const output = new Uint8Array(entry.size);
    let written = 0;
    for (const cluster of chain) {
      for (let index = 0; index < fs.sectorsPerCluster && written < output.length; index++) {
        const sector = this.readSector(this.clusterSector(cluster, index));
        const amount = Math.min(Vfat.SECTOR_SIZE, output.length - written);
        output.set(sector.subarray(0, amount), written);
        written += amount;
      }
      if (written === output.length) break;
    }
    return output;
  }

  writeFile(path: string, contents: Uint8Array): void {
    if (!this.isBytes(contents)) this.fail("file contents must be a Uint8Array");
    if (contents.length > Vfat.MAX_FILE_SIZE) this.fail("file exceeds the size limit");
    const entry = this.lookup(path);
    if ((entry.attributes & 0x01) !== 0) this.fail("file is read-only");
    const chain = this.clusterChain(entry.cluster);
    const fs = this.mount();
    const capacity = chain.length * fs.sectorsPerCluster * Vfat.SECTOR_SIZE;
    if (contents.length > capacity) this.fail("writes cannot extend a cluster chain");

    let offset = 0;
    for (const cluster of chain) {
      for (let index = 0; index < fs.sectorsPerCluster && offset < contents.length; index++) {
        const sector = new Uint8Array(Vfat.SECTOR_SIZE);
        const amount = Math.min(Vfat.SECTOR_SIZE, contents.length - offset);
        sector.set(contents.subarray(offset, offset + amount));
        this.writeSector(this.clusterSector(cluster, index), sector);
        offset += amount;
      }
      if (offset === contents.length) break;
    }

    const directorySector = this.readSector(entry.sector);
    this.write32(directorySector, entry.offset + 28, contents.length);
    this.writeSector(entry.sector, directorySector);
  }

  private uriPath(uri: string): string {
    const prefix = "virtiovfat:///";
    if (!uri.startsWith(prefix)) this.fail("expected a virtiovfat:/// URI");
    return `/${uri.slice(prefix.length)}`;
  }

  private canonicalUri(specifier: string, referrer: string): string {
    if (specifier.includes("?") || specifier.includes("#")) {
      this.fail("invalid module specifier");
    }
    let path: string;
    if (specifier.startsWith("virtiovfat:///")) {
      path = this.uriPath(specifier);
    } else if (specifier.startsWith("/")) {
      path = specifier;
    } else {
      const base = referrer ? this.uriPath(referrer) : "/app/main.js";
      path = `${base.slice(0, base.lastIndexOf("/") + 1)}${specifier}`;
    }
    const parts = this.normalizePath(path);
    return `virtiovfat:///${parts.map((part) => part.toLowerCase()).join("/")}`;
  }

  readModuleSync(specifier: string, referrer: string): VfatModuleSource {
    const url = this.canonicalUri(specifier, referrer);
    return { url, bytes: this.readFile(this.uriPath(url)) };
  }

  async readModule(
    specifier: string,
    referrer: string,
  ): Promise<VfatModuleSource> {
    return this.readModuleSync(specifier, referrer);
  }
}

const global = globalThis as VfatGlobal;
const vfat = new Vfat(global.kz.block);
global.kz.vfat = {
  readFile: (path) => vfat.readFile(path),
  writeFile: (path, contents) => vfat.writeFile(path, contents),
};
global.__kzVfatReadModule = (specifier, referrer) =>
  vfat.readModule(specifier, referrer);
global.__kzVfatReadModuleSync = (specifier, referrer) =>
  vfat.readModuleSync(specifier, referrer);
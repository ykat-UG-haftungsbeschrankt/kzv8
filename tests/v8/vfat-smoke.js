const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const sectorSize = 512;
const partitionStart = 1;
const volumeSectors = 8192;
const disk = new Uint8Array((partitionStart + volumeSectors) * sectorSize);

function set16(bytes, offset, value) {
  bytes[offset] = value & 0xff;
  bytes[offset + 1] = (value >>> 8) & 0xff;
}

function set32(bytes, offset, value) {
  bytes[offset] = value & 0xff;
  bytes[offset + 1] = (value >>> 8) & 0xff;
  bytes[offset + 2] = (value >>> 16) & 0xff;
  bytes[offset + 3] = (value >>> 24) & 0xff;
}

function writeSector(sector, bytes) {
  disk.set(bytes, sector * sectorSize);
}

let fatSectors = 1;
for (;;) {
  const clusters = volumeSectors - 1 - 2 * fatSectors - 32;
  const needed = Math.ceil((clusters + 2) * 2 / sectorSize);
  if (needed === fatSectors) break;
  fatSectors = needed;
}
const rootSector = partitionStart + 1 + 2 * fatSectors;
const dataSector = rootSector + 32;
const fatStart = partitionStart + 1;

const mbr = new Uint8Array(sectorSize);
mbr[446 + 4] = 0x06;
set32(mbr, 446 + 8, partitionStart);
set32(mbr, 446 + 12, volumeSectors);
mbr[510] = 0x55;
mbr[511] = 0xaa;
writeSector(0, mbr);

const boot = new Uint8Array(sectorSize);
boot[0] = 0xeb;
boot[1] = 0x3c;
boot[2] = 0x90;
boot.set(Buffer.from("MSWIN4.1"), 3);
set16(boot, 11, sectorSize);
boot[13] = 1;
set16(boot, 14, 1);
boot[16] = 2;
set16(boot, 17, 512);
set16(boot, 19, volumeSectors);
boot[21] = 0xf8;
set16(boot, 22, fatSectors);
set16(boot, 24, 63);
set16(boot, 26, 255);
set32(boot, 28, partitionStart);
boot[38] = 0x29;
boot.set(Buffer.from("KZVFAT16   "), 43);
boot.set(Buffer.from("FAT16   "), 54);
boot[510] = 0x55;
boot[511] = 0xaa;
writeSector(partitionStart, boot);

function setFatEntry(cluster, value) {
  for (let copy = 0; copy < 2; copy++) {
    const byteOffset = cluster * 2;
    const sector = fatStart + copy * fatSectors + Math.floor(byteOffset / sectorSize);
    const offset = byteOffset % sectorSize;
    const bytes = disk.slice(sector * sectorSize, (sector + 1) * sectorSize);
    set16(bytes, offset, value);
    writeSector(sector, bytes);
  }
}

setFatEntry(0, 0xfff8);
setFatEntry(1, 0xffff);
for (let cluster = 2; cluster <= 5; cluster++) setFatEntry(cluster, 0xffff);

function shortEntry(name, attributes, cluster, size) {
  const entry = new Uint8Array(32);
  entry.set(Buffer.from(name.padEnd(11, " ").slice(0, 11)), 0);
  entry[11] = attributes;
  set16(entry, 26, cluster);
  set32(entry, 28, size);
  return entry;
}

const root = new Uint8Array(sectorSize);
root.set(shortEntry("APP", 0x10, 2, 0), 0);
writeSector(rootSector, root);

function lfnEntries(name, shortName) {
  const short = Buffer.from(shortName, "ascii");
  let checksum = 0;
  for (let index = 0; index < 11; index++) {
    checksum = (((checksum & 1) << 7) + (checksum >>> 1) + short[index]) & 0xff;
  }
  const codes = [];
  for (let index = 0; index < name.length; index++) codes.push(name.charCodeAt(index));
  codes.push(0);
  while (codes.length % 13 !== 0) codes.push(0xffff);
  const count = codes.length / 13;
  const offsets = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
  const entries = [];
  for (let ordinal = count; ordinal >= 1; ordinal--) {
    const entry = new Uint8Array(32);
    entry.fill(0xff);
    entry[0] = ordinal | (ordinal === count ? 0x40 : 0);
    entry[11] = 0x0f;
    entry[12] = 0;
    entry[13] = checksum;
    set16(entry, 26, 0);
    for (let index = 0; index < 13; index++) {
      set16(entry, offsets[index], codes[(ordinal - 1) * 13 + index]);
    }
    entries.push(entry);
  }
  return entries;
}

const mainSource = Buffer.from(
  'import { answer } from "./feature-module.js";\nconsole.log(answer);\n',
);
const featureSource = Buffer.from("export const answer = 42;\n");
const scratchSource = Buffer.from("write-check");
const appDirectory = new Uint8Array(sectorSize);
let directoryOffset = 0;

function addAppFile(longName, shortName, cluster, contents) {
  if (longName !== shortName) {
    for (const entry of lfnEntries(longName, shortName)) {
      appDirectory.set(entry, directoryOffset);
      directoryOffset += 32;
    }
  }
  appDirectory.set(shortEntry(shortName, 0x20, cluster, contents.length), directoryOffset);
  directoryOffset += 32;
  const fileSector = dataSector + cluster - 2;
  const padded = new Uint8Array(sectorSize);
  padded.set(contents);
  writeSector(fileSector, padded);
}

addAppFile("main.js", "MAIN    JS ", 3, mainSource);
addAppFile("feature-module.js", "FEATUR~1JS ", 4, featureSource);
addAppFile("scratch.bin", "SCRATCH BIN", 5, scratchSource);
writeSector(dataSector, appDirectory);

const context = {
  kz: {
    block: {
      capacitySectors: () => BigInt(disk.length / sectorSize),
      read(sector, length) {
        const start = Number(sector) * sectorSize;
        return disk.slice(start, start + length);
      },
      write(sector, bytes) {
        const start = Number(sector) * sectorSize;
        disk.set(bytes, start);
        return bytes.length;
      },
    },
  },
};
const sourcePath = path.join(__dirname, "../../app/vfat.js");
vm.runInNewContext(fs.readFileSync(sourcePath, "utf8"), context);

async function run() {
  const main = await context.__kzVfatReadModule("virtiovfat:///app/main.js", "");
  assert.equal(main.url, "virtiovfat:///app/main.js");
  assert.equal(Buffer.from(main.bytes).toString(), mainSource.toString());

  const feature = context.__kzVfatReadModuleSync("./feature-module.js", main.url);
  assert.equal(feature.url, "virtiovfat:///app/feature-module.js");
  assert.equal(Buffer.from(feature.bytes).toString(), featureSource.toString());

  const replacement = new Uint8Array(Buffer.from("updated-data"));
  context.kz.vfat.writeFile("/app/scratch.bin", replacement);
  assert.equal(Buffer.from(context.kz.vfat.readFile("/app/scratch.bin")).toString(),
    "updated-data");

  assert.throws(() => context.kz.vfat.readFile("/private/secret.js"), /under \/app/);
  console.log("PASS: JavaScript VFAT MBR, LFN, module lookup, and file overwrite");
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
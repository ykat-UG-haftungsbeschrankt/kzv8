import { answer } from "./feature-module.js";

if (answer !== 42) throw new Error("ESM dependency returned the wrong answer");

const writePath = "/app/write-check.bin";
const original = kz.vfat.readFile(writePath);
const replacement = new Uint8Array(original.length);
replacement.set(original);
replacement[0] = 0x57;
kz.vfat.writeFile(writePath, replacement);
if (kz.vfat.readFile(writePath)[0] !== 0x57) {
	throw new Error("VFAT write did not persist");
}

Reflect.set(globalThis, "__kzAppLoaded", true);
console.log("kz: VFAT ESM app OK");
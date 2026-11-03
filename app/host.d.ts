interface KzBlockApi {
	capacitySectors(): bigint;
	read(sector: bigint, byteLength: number): Uint8Array;
	write(sector: bigint, bytes: Uint8Array): number;
}

interface KzVfatApi {
	readFile(path: string): Uint8Array;
	writeFile(path: string, contents: Uint8Array): void;
}

declare const kz: {
	block: KzBlockApi;
	vfat: KzVfatApi;
};

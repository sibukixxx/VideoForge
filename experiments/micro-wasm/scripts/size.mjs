import { readFile } from "node:fs/promises";
import { gzipSync, brotliCompressSync } from "node:zlib";

const path = new URL("../pkg/videoforge_timeline_wasm_bg.wasm", import.meta.url);
const wasm = await readFile(path);
console.log(JSON.stringify({ raw_bytes: wasm.length, gzip_bytes: gzipSync(wasm).length, brotli_bytes: brotliCompressSync(wasm).length }, null, 2));

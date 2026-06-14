import esbuild from "esbuild";
import { mkdir } from "node:fs/promises";

const outputDir = "src-tauri/generated/bridges";

const bridges = [
	["claude-sdk-bridge.mjs", "claude-sdk-bridge.bundled.mjs"],
];

await mkdir(outputDir, { recursive: true });

for (const [entry, outfile] of bridges) {
	await esbuild.build({
		entryPoints: [`src-tauri/resources/${entry}`],
		bundle: true,
		platform: "node",
		format: "esm",
		external: ["node:*"],
		minify: true,
		outfile: `${outputDir}/${outfile}`,
	});
}

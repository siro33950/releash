import esbuild from "esbuild";

await esbuild.build({
	entryPoints: ["src-tauri/resources/claude-sdk-bridge.mjs"],
	bundle: true,
	platform: "node",
	format: "esm",
	external: ["node:*"],
	minify: true,
	outfile: "src-tauri/resources/claude-sdk-bridge.bundled.mjs",
});

import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [tailwindcss(), react()],
	resolve: {
		alias: {
			"@": path.resolve(import.meta.dirname, "./src"),
		},
	},
	build: {
		outDir: "src-tauri/resources/pwa",
		emptyOutDir: true,
		rollupOptions: {
			input: path.resolve(import.meta.dirname, "pwa.html"),
		},
	},
	root: ".",
});

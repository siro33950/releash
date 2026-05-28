import path from "node:path";
import { sentryVitePlugin } from "@sentry/vite-plugin";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// https://vite.dev/config/
export default defineConfig(async () => ({
    plugins: [
        tailwindcss(),
        react(),
        // @ts-expect-error process is a nodejs global
        ...(process.env.SENTRY_AUTH_TOKEN
            ? [
                sentryVitePlugin({
                    // @ts-expect-error process is a nodejs global
                    org: process.env.SENTRY_ORG,
                    // @ts-expect-error process is a nodejs global
                    project: process.env.SENTRY_PROJECT,
                }),
            ]
            : []),
    ],
    resolve: {
        alias: {
            "@": path.resolve(import.meta.dirname, "./src"),
        },
        dedupe: ["react", "react-dom"],
    },
    build: {
        sourcemap: true,
    },
    worker: {
        format: "es",
    },
    test: {
        globals: true,
        environment: "jsdom",
        setupFiles: ["./src/test/setup.ts"],
        exclude: ["node_modules", "tests"],
        coverage: {
            provider: "v8",
            reporter: ["text", "json", "html"],
            reportsDirectory: "coverage",
        },
    },
    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
        port: 1420,
        strictPort: true,
        host: host || false,
        hmr: host
            ? {
                protocol: "ws",
                host,
                port: 1421,
            }
            : undefined,
        watch: {
            // 3. tell Vite to ignore watching `src-tauri`
            ignored: ["**/src-tauri/**"],
        },
    },
}));

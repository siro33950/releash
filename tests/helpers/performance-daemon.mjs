import { spawn } from "node:child_process";
import { mkdir, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { setTimeout } from "node:timers/promises";
import { fromBinary, fromJson, toBinary } from "@bufbuild/protobuf";
import { build } from "esbuild";

export async function startPerformanceDaemon(binary, directory, environment) {
    await mkdir(directory, { recursive: true });
    const child = spawn(resolve(binary), ["--internal-daemon", directory], {
        env: { ...process.env, ...environment },
        cwd: directory,
        stdio: ["ignore", "ignore", "inherit"],
    });
    const kill = () => {
        if (child.exitCode === null && child.signalCode === null) child.kill();
    };
    process.once("exit", kill);
    child.once("exit", () => process.removeListener("exit", kill));
    let spawnError;
    child.on("error", (error) => { spawnError = error; process.removeListener("exit", kill); });
    const exited = new Promise((done) => child.once("exit", done));
    let discovery;
    try {
        const deadline = Date.now() + 30_000;
        while (!discovery) {
            if (spawnError) throw spawnError;
            if (child.exitCode !== null || child.signalCode !== null) {
                throw new Error(`performance daemon exited before discovery: ${child.exitCode ?? child.signalCode}`);
            }
            try {
                const client = JSON.parse(await readFile(join(directory, "client-api.json"), "utf8"));
                const master = JSON.parse(await readFile(join(directory, "local-api.json"), "utf8"));
                if (client.pid === child.pid && master.pid === child.pid &&
                    client.instance_id === master.instance_id && client.port === master.port &&
                    client.process_started_at === master.process_started_at && client.token !== master.token) {
                    discovery = client;
                }
            } catch (error) {
                if (error.code !== "ENOENT") throw error;
            }
            if (Date.now() >= deadline) throw new Error("performance daemon discovery timed out");
            if (!discovery) await setTimeout(20);
        }
    } catch (error) {
        if (child.pid && child.exitCode === null && child.signalCode === null) {
            child.kill();
            await exited;
        }
        throw error;
    }
    return async () => {
        if (child.exitCode !== null || child.signalCode !== null) return;
        let socket;
        try {
            const bundle = await build({
                entryPoints: ["src/generated/client_pb.ts"], bundle: true,
                platform: "node", format: "esm", write: false,
            });
            const { EnvelopeSchema } = await import(
                `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
            );
            socket = new WebSocket(`ws://127.0.0.1:${discovery.port}/v1/client`, `releash-bearer.${discovery.token}`);
            socket.binaryType = "arraybuffer";
            const send = (body) => socket.send(toBinary(EnvelopeSchema, fromJson(EnvelopeSchema, body)));
            socket.onopen = () => send({ hello: {} });
            socket.onmessage = ({ data }) => {
                if (fromBinary(EnvelopeSchema, new Uint8Array(data)).body.case === "hello") {
                    send({ request: {
                        requestId: crypto.randomUUID(),
                        requestApplicationQuit: { request: {
                            request_id: crypto.randomUUID(), intent: { exit: { code: 0 } },
                        } },
                    } });
                }
            };
            const timeout = setTimeout(15_000, "timeout", { ref: false });
            if (await Promise.race([exited, timeout]) === "timeout") {
                throw new Error("performance daemon did not finish coordinated shutdown");
            }
            if (child.exitCode !== 0) throw new Error(`performance daemon failed: ${child.exitCode ?? child.signalCode}`);
        } finally {
            socket?.close();
            if (child.exitCode === null && child.signalCode === null) {
                child.kill();
                await exited;
            }
        }
    };
}

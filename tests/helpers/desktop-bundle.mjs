import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { setTimeout } from "node:timers/promises";
import { pathToFileURL } from "node:url";

export const dataDir = join(homedir(), "Library/Application Support/com.releash.app.performance");
export const service = `gui/${process.getuid?.()}/com.releash.app`;
export const port = 4445;
export function requireDisposableAccount() {
    assert.equal(process.platform, "darwin");
    assert.equal(process.env.RELEASH_DISPOSABLE_MACOS_ACCOUNT, "1",
        "Use a disposable macOS login account with Accessibility access for osascript; this suite changes login registration.");
}
export async function waitFor(probe, message, timeout = 30_000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
        const result = await probe();
        if (result) return result;
        await setTimeout(50);
    }
    assert.fail(message);
}
export function alive(pid) {
    try { process.kill(pid, 0); return true; }
    catch (error) { if (error.code === "ESRCH") return false; throw error; }
}
export function processes() {
    return execFileSync("/bin/ps", ["-axo", "pid=,ppid=,comm="], { encoding: "utf8" })
        .trim().split("\n").map(line => {
            const [, pid, parent, executable] = line.match(/^\s*(\d+)\s+(\d+)\s+(.+)$/);
            return { pid: Number(pid), parent: Number(parent), executable };
        });
}
export function pair(bundle) {
    const all = processes();
    const ui = all.filter(p => p.executable === join(bundle, "Contents/MacOS/releash"));
    const daemon = all.filter(p => p.executable === join(bundle, "Contents/MacOS/releash-backend"));
    if (ui.length !== 1 || daemon.length !== 1) return null;
    assert.equal(daemon[0].parent, ui[0].pid, "bundled daemon must be owned by UI");
    return { ui: ui[0].pid, daemon: daemon[0].pid };
}
export function discovery() {
    const file = join(dataDir, "client-api.json");
    return existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : null;
}
export function start(bundle, args = []) {
    const child = spawn(join(bundle, "Contents/MacOS/releash"), args, {
        cwd: bundle,
        env: { ...process.env, PATH: "/usr/bin:/bin:/usr/sbin:/sbin", TAURI_WEBDRIVER_PORT: String(port), RELEASH_PERF_REAL_APP: "1", RELEASH_TEST_CLI_INSTALL_ATTEMPT: join(dataDir, "cli-install-attempt") },
        stdio: "ignore",
    });
    child.on("error", error => { throw error; });
    child.unref();
    return child;
}
async function webdriver(path, body) {
    const response = await fetch(`http://127.0.0.1:${port}${path}`, {
        method: body === undefined ? "GET" : "POST",
        headers: { "Content-Type": "application/json" },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        signal: AbortSignal.timeout(35_000),
    });
    const result = await response.json();
    assert.ok(response.ok, JSON.stringify(result));
    return result.value;
}
export async function connect() {
    await waitFor(async () => {
        try { return (await fetch(`http://127.0.0.1:${port}/status`, { signal: AbortSignal.timeout(500) })).ok; }
        catch { return false; }
    }, "bundled UI WebDriver did not start");
    const { sessionId } = await webdriver("/session", { capabilities: { alwaysMatch: { "wdio:tauriServiceOptions": { windowLabel: "main" } } } });
    const execute = (script, args = []) => webdriver(`/session/${sessionId}/execute/sync`, { script, args });
    await waitFor(() => execute("return typeof window.__RELEASH_INVOKE_CLIENT__ === 'function'"), "renderer did not initialize");
    const call = async (kind, command, args = {}) => {
        const result = await webdriver(`/session/${sessionId}/execute/async`, {
            script: `const [kind, command, args, done] = arguments;
                const invoke = kind === 'client' ? window.__RELEASH_INVOKE_CLIENT__ : window.__TAURI__.core.invoke;
                invoke(command, args).then(value => done({ok:true, value}), error => done({ok:false, error:String(error)}));`,
            args: [kind, command, args],
        });
        assert.equal(result.ok, true, result.error);
        return result.value;
    };
    await waitFor(async () => (await call("shell", "get_daemon_status")).phase === "ready", "renderer state restoration did not complete");
    await call("client", "get_app_settings");
    return { call, execute };
}
export function accessibility(pid, script) {
    assert.ok(Number.isInteger(pid));
    return execFileSync("/usr/bin/osascript", ["-e", `tell application "System Events"
        tell (first application process whose unix id is ${pid})
            ${script}
        end tell
    end tell`], { encoding: "utf8", timeout: 10_000 }).trim();
}
export function tray(pid, item) {
    assert.ok(["Quit", "Show Releash"].includes(item));
    accessibility(pid, `click menu bar item 1 of menu bar 2
        click menu item "${item}" of menu 1 of menu bar item 1 of menu bar 2`);
}
export async function quit(bundle, client, from = "tray") {
    const current = pair(bundle);
    assert.ok(current);
    const worktreePath = join(dataDir, "quit-workflow");
    mkdirSync(worktreePath, { recursive: true });
    execFileSync("/usr/bin/git", ["init", worktreePath]);
    await client.call("client", "add_repo_path", { path: worktreePath });
    const workflowName = `releash-quit-${current.daemon}`;
    const workflowFile = join(await client.call("client", "get_automation_config_dir"), `${workflowName}.yml`);
    const marker = join(worktreePath, "command-pid");
    const stopped = join(worktreePath, "command-stopped");
    rmSync(marker, { force: true });
    rmSync(stopped, { force: true });
    writeFileSync(workflowFile, `name: ${workflowName}\ndescription: native quit acceptance\nnodes:\n  main:\n    command: trap 'printf stopped > command-stopped; exit 0' TERM; echo $$ > command-pid; while :; do sleep 1; done\n`, { flag: "wx" });
    let commandPid;
    const database = new DatabaseSync(join(dataDir, "local-event-store.sqlite3"), { readOnly: true });
    try {
        await client.call("client", "start_workflow", { workflowName, worktreePath });
        commandPid = await waitFor(() => {
            const pid = existsSync(marker) ? Number(readFileSync(marker, "utf8").trim()) : 0;
            return Number.isInteger(pid) && pid > 0 && alive(pid) ? pid : false;
        }, "quit fixture command did not start");
        if (from === "tray") tray(current.ui, "Quit");
        else if (["logout", "system-restart", "shutdown"].includes(from)) execFileSync("/usr/bin/xcrun", ["swift", "-module-cache-path", join(dataDir, "swift-cache"), resolve("tests/helpers/os-termination.swift"), String(current.ui), from], { timeout: 60_000 });
        else if (from === "cmd-q") accessibility(current.ui, 'set frontmost to true\nkeystroke "q" using command down');
        else if (from === "applescript") execFileSync("/usr/bin/osascript", ["-e", `tell application ${JSON.stringify(bundle)} to quit`], { timeout: 30_000 });
        else if (from === "dock") execFileSync("/usr/bin/osascript", ["-e", `tell application "System Events" to tell process "Dock"
            set appItem to first UI element of list 1 whose value of attribute "AXURL" is ${JSON.stringify(pathToFileURL(`${bundle}/`).href)}
            perform action "AXShowMenu" of appItem
            click menu item "Quit" of menu 1 of appItem
        end tell`], { timeout: 30_000 });
        else accessibility(current.ui, 'click (last menu item of menu 1 of menu bar item 1 of menu bar 1)');
        await waitFor(() => !alive(current.ui) && !alive(current.daemon), "Quit must stop daemon and UI");
        assert.equal(existsSync(stopped), true, `${from} must request graceful command shutdown before exiting`);
        assert.equal(alive(commandPid), false, `${from} must execute workflow command shutdown`);
        assert.equal(database.prepare("SELECT COUNT(*) AS count FROM sqlite_master WHERE name IN ('shutdown_plans', 'shutdown_targets', 'caller_attempts')").get().count, 0);
        return current;
    } finally {
        database.close();
        if (commandPid && alive(commandPid)) process.kill(-commandPid, "SIGKILL");
        rmSync(workflowFile, { force: true });
    }
}
export function launchctl(...args) {
    return spawnSync("/bin/launchctl", args, { encoding: "utf8" });
}

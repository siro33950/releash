import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { setTimeout } from "node:timers/promises";
import test from "node:test";
import { accessibility, alive, connect, dataDir, discovery, launchctl, pair, processes, quit, requireDisposableAccount, service, start, tray, waitFor } from "./helpers/desktop-bundle.mjs";

// Run after `pnpm build:desktop:acceptance` in a disposable macOS account.
// The copied .app is the only test input; no CARGO_BIN_EXE path is injected.
test("配布.appの起動・最小化・閉鎖後のworkflow継続・本番Restart・メニューとtrayのQuit", { timeout: 180_000 }, async () => {
    requireDisposableAccount();
    assert.equal(existsSync(dataDir), false, "refusing to touch existing performance data");
    assert.notEqual(launchctl("print", service).status, 0, "login item must be absent");
    const directory = realpathSync(mkdtempSync(join(tmpdir(), "releash-bundle-")));
    const bundle = join(directory, "Releash.app");
    cpSync(resolve(process.env.RELEASH_ACCEPTANCE_BUNDLE ?? "src-tauri/target/debug/bundle/macos/Releash Performance.app"), bundle, { recursive: true });
    assert.ok(existsSync(join(bundle, "Contents/MacOS/releash-backend")));
    mkdirSync(dataDir);
    writeFileSync(join(dataDir, "releash.toml"), "[app]\nclose_to_tray = true\nstart_minimized = true\n");
    let descendants = [];
    let workflowFile;
    try {
        // Given / When: first Ready with --hidden must not create a main window.
        start(bundle, ["--hidden"]);
        const first = await waitFor(() => pair(bundle), "UI did not spawn its bundled daemon");
        await waitFor(() => discovery()?.pid === first.daemon, "backend did not become ready");
        await setTimeout(2_000);
        assert.equal(accessibility(first.ui, "count windows"), "0");
        assert.equal(accessibility(first.ui, "count menu bar items of menu bar 2"), "1");
        tray(first.ui, "Show Releash");
        let client = await connect();
        const initial = discovery();
        assert.equal(existsSync(join(dataDir, "cli-install-attempt")), false, "startup reached the actual CLI installation boundary");
        // Given: a real workflow remains running until released after reopening.
        const worktreePath = join(directory, "close-workflow");
        execFileSync("/usr/bin/git", ["init", worktreePath]);
        await client.call("client", "add_repo_path", { path: worktreePath });
        const workflowName = basename(directory);
        const sourceFile = join(await client.call("client", "get_automation_config_dir"), `${workflowName}.yml`);
        writeFileSync(sourceFile, `name: ${workflowName}\ndescription: window close acceptance\nnodes:\n  main:\n    command: printf running > window-close-running; while [ ! -f window-close-finish ]; do sleep 0.1; done\n`, { flag: "wx" });
        workflowFile = sourceFile;
        const executionId = await client.call("client", "start_workflow", { workflowName, worktreePath });
        const workflowStatus = () => JSON.parse(execFileSync(join(bundle, "Contents/MacOS/releash-backend"), ["workflow", "status", executionId, "--json"], { encoding: "utf8", timeout: 10_000, env: { ...process.env, RELEASH_DATA_DIR: dataDir } })).status;
        await waitFor(() => existsSync(join(worktreePath, "window-close-running")), "workflow did not start");
        assert.equal(workflowStatus(), "running");
        // When: click the native close button, delivering CloseRequested.
        accessibility(first.ui, 'click (first button of window 1 whose subrole is "AXCloseButton")');
        await waitFor(() => accessibility(first.ui, "count windows") === "0", "close did not hide the window");
        // Then: the same UI, daemon, tray and workflow survive without a shutdown request.
        assert.deepEqual(pair(bundle), first);
        assert.deepEqual(discovery(), initial);
        assert.equal(accessibility(first.ui, "count menu bar items of menu bar 2"), "1");
        assert.equal((await client.call("shell", "get_daemon_status")).phase, "ready");
        assert.equal(workflowStatus(), "running");
        tray(first.ui, "Show Releash");
        await waitFor(() => accessibility(first.ui, "count windows") === "1", "closed window did not reopen");
        client = await connect();
        assert.deepEqual(pair(bundle), first);
        assert.equal(workflowStatus(), "running");
        writeFileSync(join(worktreePath, "window-close-finish"), "");
        await waitFor(async () => workflowStatus() === "completed", "workflow did not complete after reopening");
        await client.call("client", "update_external_editor", { editor: "bundle-restart-marker" });
        // When: the real command goes through observe -> spawn_successor -> wait_for_predecessor.
        await client.execute("setTimeout(() => window.__TAURI__.core.invoke('restart_desktop'), 100); return true");
        const second = await waitFor(() => {
            const current = pair(bundle);
            if (!current || current.ui === first.ui || current.daemon === first.daemon) return false;
            assert.equal(alive(first.ui), false, "successor started backend before predecessor exited");
            assert.equal(alive(first.daemon), false, "old and new backend overlap");
            return current;
        }, "successor UI and daemon did not start");
        await waitFor(() => discovery()?.pid === second.daemon, "successor backend is not ready");
        await setTimeout(2_000);
        assert.equal(accessibility(second.ui, "count windows"), "0", "--hidden must survive Restart");
        tray(second.ui, "Show Releash");
        client = await connect();
        assert.notEqual(discovery().instance_id, initial.instance_id);
        assert.equal((await client.call("client", "get_app_settings")).external_editor, "bundle-restart-marker");
        // Then: both menu routes terminate the real UI and its coordinated daemon.
        await quit(bundle, client, "tray");
        start(bundle);
        await waitFor(() => pair(bundle), "relaunch after Quit failed");
        client = await connect();
        assert.equal((await client.call("client", "get_app_settings")).external_editor, "bundle-restart-marker");
        await quit(bundle, client, "menu");
        for (const route of ["cmd-q", "dock", "applescript", "logout", "system-restart", "shutdown"]) {
            start(bundle);
            await waitFor(() => pair(bundle), "native Quit relaunch failed");
            client = await connect();
            await quit(bundle, client, route);
        }
        start(bundle);
        const crashing = await waitFor(() => pair(bundle), "UI crash fixture did not start");
        client = await connect();
        await client.call("client", "get_or_spawn_terminal_surface", { rows: 24, cols: 80, cwd: directory, owner: {kind: "workspace", workspacePath: directory}, startupCommand: "sleep 100 & echo $! > grandchild-pid; printf '%s' \"$$\" > child-pid" });
        await waitFor(() => existsSync(join(directory, "grandchild-pid")) && existsSync(join(directory, "child-pid")), "terminal descendants did not start");
        descendants = ["child-pid", "grandchild-pid"].map(file => Number(readFileSync(join(directory, file), "utf8").trim()));
        assert.ok(descendants.every(pid => pid > 0 && alive(pid)));
        process.kill(crashing.ui, "SIGKILL");
        await waitFor(() => [crashing.ui, crashing.daemon, ...descendants].every(pid => !alive(pid)), "UI crash left daemon descendants running");
        await setTimeout(8_000);
        assert.deepEqual(processes().filter(p => p.executable.startsWith(`${bundle}/`)), [], "UI or daemon restarted without opening the app");
        assert.equal(existsSync(join(dataDir, "cli-install-attempt")), false);
        await setTimeout(1_200);
        assert.equal(pair(bundle), null);
        assert.ok(readFileSync(join(bundle, "Contents/Library/LaunchAgents/com.releash.app.plist"), "utf8").includes("BundleProgram"));
    } finally {
        const owned = processes().filter(p => p.executable.startsWith(`${bundle}/`) || descendants.includes(p.pid));
        for (const process of owned) if (alive(process.pid)) globalThis.process.kill(process.pid, "SIGKILL");
        await waitFor(() => owned.every(p => !alive(p.pid)), "test-owned processes did not exit");
        if (workflowFile) rmSync(workflowFile, { force: true });
        rmSync(directory, { recursive: true, force: true });
        rmSync(dataDir, { recursive: true, force: true });
    }
});

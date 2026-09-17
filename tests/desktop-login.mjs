// Disposable macOS account only. Build/copy the acceptance .app first.
// Run prepare, log out/in, run enabled, log out/in, run disabled,
// log out/in, run deleted. Every verification rejects the same login session.
import assert from "node:assert/strict";
import { cpSync, existsSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { setTimeout } from "node:timers/promises";
import { accessibility, connect, dataDir, discovery, launchctl, pair, processes, quit, requireDisposableAccount, service, start, tray, waitFor } from "./helpers/desktop-bundle.mjs";

requireDisposableAccount();
const phase = process.argv[2];
assert.ok(["prepare", "enabled", "disabled", "deleted"].includes(phase), "expected prepare | enabled | disabled | deleted");
const directory = resolve("test-results/desktop-login");
const statePath = join(directory, "state.json");
const domain = `gui/${process.getuid()}`;
function loginSession() {
    const result = launchctl("print", domain);
    assert.equal(result.status, 0, result.stderr);
    const match = result.stdout.match(/\basid = (\d+)/);
    assert.ok(match, "login audit session is unavailable");
    return match[1];
}
function save(next, bundle) {
    writeFileSync(statePath, JSON.stringify({ phase: next, bundle, login: loginSession() }, null, 2));
    console.log(`PASS ${phase}; log out/in, then run: pnpm test:desktop:login ${next}`);
}
function singleRegistration() {
    const result = launchctl("list");
    assert.equal(result.status, 0, result.stderr);
    const registrations = result.stdout.split("\n").filter(line => /\s(?:com\.releash\..*|releash)$/.test(line));
    assert.equal(registrations.length, 1, registrations.join("\n"));
    assert.ok(registrations[0].endsWith("com.releash.app"));
}
async function absent(bundle) {
    for (let i = 0; i < 100; i++) {
        assert.equal(processes().filter(p => p.executable.startsWith(`${bundle}/`)).length, 0);
        await setTimeout(100);
    }
    const status = launchctl("print", service);
    if (status.status === 0) {
        assert.match(status.stdout, /runs = 0\b/, "launchd attempted to start the removed/disabled app");
        assert.doesNotMatch(status.stdout, /\bpid = \d+/);
    }
}

if (phase === "prepare") {
    assert.equal(existsSync(statePath), false, "previous acceptance run must be inspected first");
    assert.equal(existsSync(dataDir), false, "refusing existing performance data");
    assert.notEqual(launchctl("print", service).status, 0, "refusing existing registration");
    mkdirSync(directory, { recursive: true });
    const bundle = join(realpathSync(directory), "Releash.app");
    cpSync(resolve(process.env.RELEASH_ACCEPTANCE_BUNDLE ?? "src-tauri/target/debug/bundle/macos/Releash Performance.app"), bundle, { recursive: true });
    mkdirSync(dataDir);
    writeFileSync(join(dataDir, "releash.toml"), "[app]\nclose_to_tray = true\nstart_minimized = true\n");
    const plist = readFileSync(join(bundle, "Contents/Library/LaunchAgents/com.releash.app.plist"), "utf8");
    assert.match(plist, /<key>RunAtLoad<\/key>\s*<true\s*\/>/);
    assert.ok(!plist.includes("KeepAlive"));
    start(bundle, ["--hidden"]);
    const current = await waitFor(() => pair(bundle), "UI and daemon did not start");
    await waitFor(() => discovery()?.pid === current.daemon, "daemon did not become Ready");
    assert.equal(accessibility(current.ui, "count windows"), "0");
    tray(current.ui, "Show Releash");
    const client = await connect();
    const registered = await client.call("shell", "set_login_item_enabled", { enabled: true });
    assert.equal(registered.requiresApproval, false, "Approve Releash in System Settings before this login acceptance run");
    assert.equal(registered.enabled, true);
    assert.equal((await client.call("client", "get_app_settings")).auto_launch, true);
    singleRegistration();
    await quit(bundle);
    save("enabled", bundle);
} else {
    const state = JSON.parse(readFileSync(statePath, "utf8"));
    assert.equal(state.phase, phase);
    assert.notEqual(loginSession(), state.login, "a real logout/login is required");
    const bundle = state.bundle;
    if (phase === "enabled") {
        // Nothing in this stage launches the app; only launchd can have started it.
        const current = await waitFor(() => pair(bundle), "login did not start UI and daemon");
        await waitFor(() => discovery()?.pid === current.daemon, "login daemon is not Ready");
        await setTimeout(2_000);
        assert.equal(accessibility(current.ui, "count windows"), "0");
        assert.equal(accessibility(current.ui, "count menu bar items of menu bar 2"), "1");
        singleRegistration();
        tray(current.ui, "Show Releash");
        const client = await connect();
        assert.equal((await client.call("shell", "get_login_item_status")).enabled, true);
        await client.call("shell", "set_login_item_enabled", { enabled: false });
        assert.equal((await client.call("client", "get_app_settings")).auto_launch, false);
        assert.equal((await client.call("shell", "get_login_item_status")).enabled, false);
        await quit(bundle);
        save("disabled", bundle);
    } else if (phase === "disabled") {
        await absent(bundle);
        start(bundle);
        await waitFor(() => pair(bundle), "manual launch failed");
        const client = await connect();
        assert.equal((await client.call("shell", "get_login_item_status")).enabled, false);
        await client.call("shell", "set_login_item_enabled", { enabled: true });
        assert.equal((await client.call("shell", "get_login_item_status")).enabled, true);
        singleRegistration();
        await quit(bundle);
        // Delete only this test's copied .app; do not unregister to mask OS behavior.
        assert.equal(bundle, join(realpathSync(directory), "Releash.app"));
        rmSync(bundle, { recursive: true });
        save("deleted", bundle);
    } else {
        assert.equal(existsSync(bundle), false);
        await absent(bundle);
        writeFileSync(statePath, JSON.stringify({ ...state, phase: "passed", login: loginSession() }, null, 2));
        console.log("PASS enabled/disabled login, minimized launch, single registration, and no launch attempts after app removal");
    }
}

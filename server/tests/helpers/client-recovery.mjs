import assert from "node:assert/strict";

const request = async (url, method, body) => {
    const response = await fetch(`${url}/releash.client.v1.ClientService/${method}`, {
        method: "POST",
        headers: { "Content-Type": "application/json", "Connect-Protocol-Version": "1" },
        body: JSON.stringify(body),
    });
    assert.equal(response.status, 200, await response.text());
};
const [first, recovered] = process.argv.slice(2);
await request(first, "UpdateCrashReporting", { enabled: true });
await request(first, "ReportMountedXtermCount", { count: "3" });
await request(recovered, "UpdateCrashReporting", { enabled: false });

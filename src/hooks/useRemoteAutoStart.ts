import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react";
import type { RemoteConfig } from "@/hooks/useRemoteConfig";

interface DetectedInterface {
	name: string;
	ip: string;
	kind: "vpn" | "lan";
}

interface ServerInfo {
	running: boolean;
	bound_ip: string | null;
	connection_mode: "vpn" | "lan" | null;
}

export function useRemoteAutoStart(ready: boolean) {
	const attempted = useRef(false);

	useEffect(() => {
		if (!ready || attempted.current) return;
		attempted.current = true;

		(async () => {
			try {
				const config = await invoke<RemoteConfig>("get_remote_config");
				if (!config.auto_start) return;

				const info = await invoke<ServerInfo>("get_server_info");
				if (info.running) return;

				const interfaces =
					await invoke<DetectedInterface[]>("get_network_info");
				const vpn = interfaces.find((i) => i.kind === "vpn");
				const lan = interfaces.find((i) => i.kind === "lan");

				let bindIp: string | undefined;
				if (vpn) {
					bindIp = vpn.ip;
				} else if (config.auto_start_on_lan && lan) {
					bindIp = lan.ip;
				}

				if (!bindIp) return;

				await invoke("start_server", {
					bindIp,
				});
			} catch (e) {
				console.warn("Remote auto-start failed:", e);
			}
		})();
	}, [ready]);
}

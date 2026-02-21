import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { formatRemoteServerError } from "@/lib/errorHandler";
import { trackEvent } from "@/lib/telemetry";

interface QrCodeResult {
	url: string;
	svg: string;
	token_svg: string;
}

interface ServerConfig {
	port: number;
	token: string;
}

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

interface StartServerResult {
	ip: string;
	mode: "vpn" | "lan";
}

export function useRemoteServer() {
	const [running, setRunning] = useState(false);
	const [qrData, setQrData] = useState<QrCodeResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [config, setConfig] = useState<ServerConfig | null>(null);
	const [interfaces, setInterfaces] = useState<DetectedInterface[]>([]);
	const [selectedIp, setSelectedIp] = useState<string | null>(null);
	const [boundIp, setBoundIp] = useState<string | null>(null);
	const [connectionMode, setConnectionMode] = useState<"vpn" | "lan" | null>(
		null,
	);
	const [showLanConfirm, setShowLanConfirm] = useState(false);
	const [pendingRepoPaths, setPendingRepoPaths] = useState<string[] | null>(
		null,
	);

	const refreshConfig = useCallback(async () => {
		try {
			const cfg = await invoke<ServerConfig>("get_server_config");
			setConfig({ port: cfg.port, token: cfg.token });
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	const refreshNetwork = useCallback(async () => {
		try {
			const detected = await invoke<DetectedInterface[]>("get_network_info");
			setInterfaces(detected);
			setSelectedIp((prev) => {
				if (prev && detected.some((i) => i.ip === prev)) return prev;
				const vpn = detected.find((i) => i.kind === "vpn");
				return vpn ? vpn.ip : (detected[0]?.ip ?? null);
			});
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	const refreshStatus = useCallback(async () => {
		try {
			const info = await invoke<ServerInfo>("get_server_info");
			setRunning(info.running);
			if (info.running) {
				setBoundIp(info.bound_ip);
				setConnectionMode(info.connection_mode);
			}
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	useEffect(() => {
		refreshConfig();
		refreshNetwork();
		refreshStatus();
	}, [refreshConfig, refreshNetwork, refreshStatus]);

	const refreshQr = useCallback(async () => {
		try {
			const result = await invoke<QrCodeResult>("get_connection_qr");
			setQrData(result);
			setError(null);
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	useEffect(() => {
		if (running && !qrData) {
			refreshQr();
		}
	}, [running, qrData, refreshQr]);

	const doStartServer = useCallback(
		async (repoPaths: string[], bindIp: string) => {
			setError(null);
			try {
				const result = await invoke<StartServerResult>("start_server", {
					repoPaths,
					bindIp,
				});
				setRunning(true);
				setBoundIp(result.ip);
				setConnectionMode(result.mode);
				trackEvent("remote_server_started");
				await refreshQr();
			} catch (e) {
				setError(formatRemoteServerError(e));
			}
		},
		[refreshQr],
	);

	const startServer = useCallback(
		async (repoPaths: string[]) => {
			if (!selectedIp) {
				setError("Please select an IP address");
				return;
			}
			const selected = interfaces.find((i) => i.ip === selectedIp);
			if (selected?.kind === "lan") {
				setPendingRepoPaths(repoPaths);
				setShowLanConfirm(true);
				return;
			}
			await doStartServer(repoPaths, selectedIp);
		},
		[selectedIp, interfaces, doStartServer],
	);

	const confirmLanStart = useCallback(async () => {
		setShowLanConfirm(false);
		if (!pendingRepoPaths || !selectedIp) return;
		await doStartServer(pendingRepoPaths, selectedIp);
		setPendingRepoPaths(null);
	}, [pendingRepoPaths, selectedIp, doStartServer]);

	const cancelLanStart = useCallback(() => {
		setShowLanConfirm(false);
		setPendingRepoPaths(null);
	}, []);

	const updateRepoPaths = useCallback(async (repoPaths: string[]) => {
		try {
			await invoke("update_server_repo_paths", { repoPaths });
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	const updateTerminalStartupCommand = useCallback(async (command: string) => {
		try {
			await invoke("update_terminal_startup_command", { command });
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	const stopServer = useCallback(async () => {
		setError(null);
		try {
			await invoke("stop_server");
			setRunning(false);
			setQrData(null);
			setBoundIp(null);
			setConnectionMode(null);
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, []);

	const updatePort = useCallback(
		async (port: number) => {
			setError(null);
			try {
				await invoke("update_server_port", { port });
				await refreshConfig();
			} catch (e) {
				setError(formatRemoteServerError(e));
			}
		},
		[refreshConfig],
	);

	const regenerateToken = useCallback(async () => {
		setError(null);
		try {
			const token = await invoke<string>("regenerate_token");
			setConfig((prev) => (prev ? { ...prev, token } : null));
			await refreshQr();
		} catch (e) {
			setError(formatRemoteServerError(e));
		}
	}, [refreshQr]);

	return {
		running,
		qrData,
		error,
		config,
		interfaces,
		selectedIp,
		setSelectedIp,
		boundIp,
		connectionMode,
		showLanConfirm,
		startServer,
		stopServer,
		confirmLanStart,
		cancelLanStart,
		refreshQr,
		refreshStatus,
		updatePort,
		regenerateToken,
		updateRepoPaths,
		updateTerminalStartupCommand,
	};
}

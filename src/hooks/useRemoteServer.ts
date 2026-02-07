import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

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
	const [pendingRootPath, setPendingRootPath] = useState<string | null>(null);

	const refreshConfig = useCallback(async () => {
		try {
			const cfg = await invoke<ServerConfig>("get_server_config");
			setConfig({ port: cfg.port, token: cfg.token });
		} catch (e) {
			setError(String(e));
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
			setError(String(e));
		}
	}, []);

	useEffect(() => {
		refreshConfig();
		refreshNetwork();
	}, [refreshConfig, refreshNetwork]);

	const refreshStatus = useCallback(async () => {
		try {
			const status = await invoke<boolean>("get_server_status");
			setRunning(status);
		} catch (e) {
			setError(String(e));
		}
	}, []);

	const refreshQr = useCallback(async () => {
		try {
			const result = await invoke<QrCodeResult>("get_connection_qr");
			setQrData(result);
			setError(null);
		} catch (e) {
			setError(String(e));
		}
	}, []);

	const doStartServer = useCallback(
		async (rootPath: string, bindIp: string) => {
			setError(null);
			try {
				const result = await invoke<StartServerResult>("start_server", {
					rootPath,
					bindIp,
				});
				setRunning(true);
				setBoundIp(result.ip);
				setConnectionMode(result.mode);
				await refreshQr();
			} catch (e) {
				setError(String(e));
			}
		},
		[refreshQr],
	);

	const startServer = useCallback(
		async (rootPath: string) => {
			if (!selectedIp) {
				setError("IPアドレスを選択してください");
				return;
			}
			const selected = interfaces.find((i) => i.ip === selectedIp);
			if (selected?.kind === "lan") {
				setPendingRootPath(rootPath);
				setShowLanConfirm(true);
				return;
			}
			await doStartServer(rootPath, selectedIp);
		},
		[selectedIp, interfaces, doStartServer],
	);

	const confirmLanStart = useCallback(async () => {
		setShowLanConfirm(false);
		if (!pendingRootPath || !selectedIp) return;
		await doStartServer(pendingRootPath, selectedIp);
		setPendingRootPath(null);
	}, [pendingRootPath, selectedIp, doStartServer]);

	const cancelLanStart = useCallback(() => {
		setShowLanConfirm(false);
		setPendingRootPath(null);
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
			setError(String(e));
		}
	}, []);

	const updatePort = useCallback(
		async (port: number) => {
			setError(null);
			try {
				await invoke("update_server_port", { port });
				await refreshConfig();
			} catch (e) {
				setError(String(e));
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
			setError(String(e));
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
	};
}

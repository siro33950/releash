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

interface NetworkInfo {
	meshnet: string | null;
	lan: string | null;
}

export function useRemoteServer() {
	const [running, setRunning] = useState(false);
	const [qrData, setQrData] = useState<QrCodeResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [config, setConfig] = useState<ServerConfig | null>(null);
	const [network, setNetwork] = useState<NetworkInfo | null>(null);
	const [boundIp, setBoundIp] = useState<string | null>(null);

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
			const info = await invoke<NetworkInfo>("get_network_info");
			setNetwork(info);
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

	const startServer = useCallback(
		async (rootPath: string) => {
			setError(null);
			try {
				const ip = await invoke<string>("start_server", { rootPath });
				setRunning(true);
				setBoundIp(ip);
				await refreshQr();
			} catch (e) {
				setError(String(e));
			}
		},
		[refreshQr],
	);

	const stopServer = useCallback(async () => {
		setError(null);
		try {
			await invoke("stop_server");
			setRunning(false);
			setQrData(null);
			setBoundIp(null);
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
		network,
		boundIp,
		startServer,
		stopServer,
		refreshQr,
		refreshStatus,
		updatePort,
		regenerateToken,
	};
}

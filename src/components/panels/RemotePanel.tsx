import { useEffect, useState } from "react";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useRemoteServer } from "@/hooks/useRemoteServer";
import { NetworkConfig } from "./NetworkConfig";
import { QrDisplay } from "./QrDisplay";
import { ServerControl } from "./ServerControl";
import { ServerSettings } from "./ServerSettings";

export interface RemotePanelProps {
	rootPaths: string[];
	terminalStartupCommand: string;
}

export function RemotePanel({
	rootPaths,
	terminalStartupCommand,
}: RemotePanelProps) {
	const {
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
		refreshStatus,
		updatePort,
		regenerateToken,
		updateTerminalStartupCommand,
	} = useRemoteServer();

	const [portInput, setPortInput] = useState("");

	useEffect(() => {
		refreshStatus();
	}, [refreshStatus]);

	const [prevConfigPort, setPrevConfigPort] = useState<number | null>(null);
	if (config && config.port !== prevConfigPort) {
		setPrevConfigPort(config.port);
		setPortInput(String(config.port));
	}

	const handleToggle = async () => {
		if (running) {
			await stopServer();
		} else if (rootPaths.length > 0) {
			await startServer(rootPaths);
			updateTerminalStartupCommand(terminalStartupCommand);
		}
	};

	const handleCopyUrl = async () => {
		if (qrData?.url) {
			await navigator.clipboard.writeText(qrData.url);
		}
	};

	const handleCopyToken = async () => {
		if (config?.token) {
			await navigator.clipboard.writeText(config.token);
		}
	};

	const handlePortBlur = async () => {
		const port = Number(portInput);
		if (config && port !== config.port && port >= 1024 && port <= 65535) {
			await updatePort(port);
		} else if (config) {
			setPortInput(String(config.port));
		}
	};

	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					Remote
				</span>
			</div>

			<ScrollArea className="flex-1 min-h-0">
				<div className="px-3 py-3 flex flex-col gap-4">
					<ServerControl
						running={running}
						error={error}
						connectionMode={connectionMode}
						rootPaths={rootPaths}
						onToggle={handleToggle}
					/>

					<NetworkConfig
						interfaces={interfaces}
						selectedIp={selectedIp}
						setSelectedIp={setSelectedIp}
						running={running}
						boundIp={boundIp}
					/>

					{running && qrData && (
						<QrDisplay
							qrData={qrData}
							config={config}
							onCopyUrl={handleCopyUrl}
							onCopyToken={handleCopyToken}
							onRegenerateToken={regenerateToken}
						/>
					)}

					{config && (
						<ServerSettings
							portInput={portInput}
							running={running}
							onPortInputChange={setPortInput}
							onPortBlur={handlePortBlur}
						/>
					)}

					{rootPaths.length === 0 && (
						<p className="text-xs text-muted-foreground">
							フォルダを開いてからサーバーを起動してください
						</p>
					)}
				</div>
			</ScrollArea>

			<AlertDialog
				open={showLanConfirm}
				onOpenChange={(o) => !o && cancelLanStart()}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>LAN接続の確認</AlertDialogTitle>
						<AlertDialogDescription>
							LAN
							IPで起動すると、同一ネットワーク上のすべてのデバイスからアクセス可能になります。安全のため、Tailscale等のメッシュVPN経由での接続を強く推奨します。それでもLANで続行しますか？
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel onClick={cancelLanStart}>
							キャンセル
						</AlertDialogCancel>
						<AlertDialogAction onClick={confirmLanStart}>
							続行
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

import { Copy, RefreshCw } from "lucide-react";
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
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useRemoteServer } from "@/hooks/useRemoteServer";

export interface RemotePanelProps {
	rootPath: string | null;
}

export function RemotePanel({ rootPath }: RemotePanelProps) {
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
	} = useRemoteServer();

	const [portInput, setPortInput] = useState("");

	useEffect(() => {
		refreshStatus();
	}, [refreshStatus]);

	useEffect(() => {
		if (config) {
			setPortInput(String(config.port));
		}
	}, [config]);

	const handleToggle = async () => {
		if (running) {
			await stopServer();
		} else if (rootPath) {
			await startServer(rootPath);
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
					{/* Server Control */}
					<div className="flex flex-col gap-2">
						<div className="flex items-center gap-2">
							<div
								className={`size-2 rounded-full ${running ? "bg-green-500" : "bg-muted-foreground"}`}
							/>
							<span className="text-xs text-muted-foreground">
								{running ? "Running" : "Stopped"}
							</span>
						</div>
						<Button
							size="sm"
							variant={running ? "destructive" : "default"}
							className="w-full text-xs"
							onClick={handleToggle}
							disabled={!running && !rootPath}
						>
							{running ? "Stop Server" : "Start Server"}
						</Button>
					</div>

					{/* Error */}
					{error && (
						<div className="text-xs text-destructive bg-destructive/10 rounded px-2 py-1.5 break-all">
							{error}
						</div>
					)}

					{/* LAN Mode Warning */}
					{running && connectionMode === "lan" && (
						<div className="text-xs text-yellow-200 bg-yellow-900/40 border border-yellow-700/50 rounded px-2 py-1.5">
							LAN接続モード — 同一ネットワーク上のデバイスがアクセス可能です
						</div>
					)}

					{/* Network */}
					<div className="flex flex-col gap-1.5 border-t border-border pt-3">
						<span className="text-xs font-medium text-muted-foreground">
							Network
						</span>
						<div className="flex flex-col gap-0.5 bg-muted rounded px-2 py-1.5">
							{interfaces.length === 0 && (
								<span className="text-[10px] text-muted-foreground">
									ネットワークが検出されません
								</span>
							)}
							{interfaces.map((iface) => (
								<label
									key={iface.ip}
									className={`flex items-center gap-2 py-0.5 cursor-pointer ${running ? "opacity-50 pointer-events-none" : ""}`}
								>
									<input
										type="radio"
										name="bind-ip"
										value={iface.ip}
										checked={selectedIp === iface.ip}
										onChange={() => setSelectedIp(iface.ip)}
										disabled={running}
										className="accent-primary size-3"
									/>
									<span className="text-[10px] text-muted-foreground uppercase w-8 shrink-0">
										{iface.kind === "vpn" ? "VPN" : "LAN"}
									</span>
									<span className="text-[10px] text-muted-foreground truncate">
										{iface.name}
									</span>
									<span className="text-[10px] font-mono text-foreground ml-auto shrink-0">
										{iface.ip}
									</span>
								</label>
							))}
							{running && boundIp && (
								<div className="flex justify-between items-center pt-1 border-t border-border/50">
									<span className="text-[10px] text-muted-foreground">
										Bind
									</span>
									<span className="text-[10px] font-mono text-foreground">
										{boundIp}
									</span>
								</div>
							)}
						</div>
					</div>

					{/* Connection QR */}
					{running && qrData && (
						<div className="flex flex-col gap-2 border-t border-border pt-3">
							<span className="text-xs font-medium text-muted-foreground">
								Connection
							</span>
							<div
								className="w-full flex justify-center"
								// biome-ignore lint/security/noDangerouslySetInnerHtml: QR SVG from trusted backend
								dangerouslySetInnerHTML={{ __html: qrData.svg }}
							/>
							<div className="flex items-center gap-1">
								<span className="flex-1 text-[10px] text-muted-foreground font-mono truncate">
									{qrData.url}
								</span>
								<Button
									variant="ghost"
									size="icon"
									className="size-5 shrink-0"
									onClick={handleCopyUrl}
								>
									<Copy className="size-3" />
								</Button>
							</div>
						</div>
					)}

					{/* Auth Token QR */}
					{running && qrData && config && (
						<div className="flex flex-col gap-2 border-t border-border pt-3">
							<span className="text-xs font-medium text-muted-foreground">
								Auth Token
							</span>
							<div
								className="w-full flex justify-center"
								// biome-ignore lint/security/noDangerouslySetInnerHtml: QR SVG from trusted backend
								dangerouslySetInnerHTML={{ __html: qrData.token_svg }}
							/>
							<div className="flex items-center gap-1">
								<span className="flex-1 text-[10px] text-muted-foreground font-mono truncate bg-muted border border-border rounded px-2 py-1">
									{config.token.slice(0, 8)}...
								</span>
								<Button
									variant="ghost"
									size="icon"
									className="size-5 shrink-0"
									onClick={handleCopyToken}
									title="Copy token"
								>
									<Copy className="size-3" />
								</Button>
								<Button
									variant="ghost"
									size="icon"
									className="size-5 shrink-0"
									onClick={regenerateToken}
									title="Regenerate token"
								>
									<RefreshCw className="size-3" />
								</Button>
							</div>
						</div>
					)}

					{/* Server Settings */}
					{config && (
						<div className="flex flex-col gap-3 border-t border-border pt-3">
							<span className="text-xs font-medium text-muted-foreground">
								Settings
							</span>

							{/* Port */}
							<div className="flex flex-col gap-1">
								<label
									htmlFor="remote-port"
									className="text-[10px] text-muted-foreground"
								>
									Port
								</label>
								<input
									id="remote-port"
									type="number"
									min={1024}
									max={65535}
									value={portInput}
									onChange={(e) => setPortInput(e.target.value)}
									onBlur={handlePortBlur}
									disabled={running}
									className="w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
								/>
							</div>
						</div>
					)}

					{/* No root path warning */}
					{!rootPath && (
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

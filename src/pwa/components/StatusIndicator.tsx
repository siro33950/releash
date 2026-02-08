import type { ConnectionStatus } from "../hooks/useWebSocket";

const statusConfig: Record<ConnectionStatus, { label: string; color: string }> =
	{
		connected: { label: "接続中", color: "bg-green-500" },
		connecting: { label: "接続試行中", color: "bg-yellow-500 animate-pulse" },
		authenticating: { label: "認証中", color: "bg-yellow-500 animate-pulse" },
		disconnected: { label: "切断", color: "bg-red-500" },
	};

interface StatusIndicatorProps {
	status: ConnectionStatus;
}

export function StatusIndicator({ status }: StatusIndicatorProps) {
	const config = statusConfig[status];
	return (
		<div className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-neutral-800 text-sm">
			<span className={`w-2 h-2 rounded-full ${config.color}`} />
			<span className="text-neutral-300">{config.label}</span>
		</div>
	);
}

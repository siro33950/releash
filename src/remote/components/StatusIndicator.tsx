import type { ConnectionStatus } from "../hooks/useWebSocket";

const statusConfig: Record<ConnectionStatus, { label: string; color: string }> =
	{
		connected: { label: "Connected", color: "bg-success" },
		connecting: { label: "Connecting", color: "bg-warning animate-pulse" },
		authenticating: {
			label: "Authenticating",
			color: "bg-warning animate-pulse",
		},
		disconnected: { label: "Disconnected", color: "bg-destructive" },
	};

interface StatusIndicatorProps {
	status: ConnectionStatus;
}

export function StatusIndicator({ status }: StatusIndicatorProps) {
	const config = statusConfig[status];
	return (
		<div className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-secondary text-sm">
			<span className={`w-2 h-2 rounded-full ${config.color}`} />
			<span className="text-secondary-foreground">{config.label}</span>
		</div>
	);
}

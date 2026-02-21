import { Button } from "@/components/ui/button";
import { Message } from "@/components/ui/message";

interface ServerControlProps {
	running: boolean;
	error: string | null;
	connectionMode: "vpn" | "lan" | null;
	rootPaths: string[];
	onToggle: () => void;
}

export function ServerControl({
	running,
	error,
	connectionMode,
	rootPaths,
	onToggle,
}: ServerControlProps) {
	return (
		<>
			<div className="flex flex-col gap-2">
				<div className="flex items-center gap-2">
					<div
						className={`size-2 rounded-full ${running ? "bg-success" : "bg-muted-foreground"}`}
					/>
					<span className="text-xs text-muted-foreground">
						{running ? "Running" : "Stopped"}
					</span>
				</div>
				<Button
					size="sm"
					variant={running ? "destructive" : "default"}
					className="w-full text-xs"
					onClick={onToggle}
					disabled={!running && rootPaths.length === 0}
				>
					{running ? "Stop Server" : "Start Server"}
				</Button>
			</div>

			{error && (
				<Message
					message={error}
					className="bg-destructive/10 rounded px-2 py-1.5"
				/>
			)}

			{running && connectionMode === "lan" && (
				<Message
					severity="warning"
					message="LAN接続モード — 同一ネットワーク上のデバイスがアクセス可能です"
					className="bg-warning/10 border border-warning/30 rounded px-2 py-1.5"
				/>
			)}
		</>
	);
}

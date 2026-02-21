export interface DetectedInterface {
	name: string;
	ip: string;
	kind: "vpn" | "lan";
}

interface NetworkConfigProps {
	interfaces: DetectedInterface[];
	selectedIp: string | null;
	setSelectedIp: (ip: string) => void;
	running: boolean;
	boundIp: string | null;
}

export function NetworkConfig({
	interfaces,
	selectedIp,
	setSelectedIp,
	running,
	boundIp,
}: NetworkConfigProps) {
	return (
		<div className="flex flex-col gap-1.5 border-t border-border pt-3">
			<span className="text-xs font-medium text-muted-foreground">Network</span>
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
						<span className="text-[10px] text-muted-foreground">Bind</span>
						<span className="text-[10px] font-mono text-foreground">
							{boundIp}
						</span>
					</div>
				)}
			</div>
		</div>
	);
}

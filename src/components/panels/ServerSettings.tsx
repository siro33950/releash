import { Input } from "@/components/ui/input";

interface ServerSettingsProps {
	portInput: string;
	running: boolean;
	onPortInputChange: (value: string) => void;
	onPortBlur: () => void;
}

export function ServerSettings({
	portInput,
	running,
	onPortInputChange,
	onPortBlur,
}: ServerSettingsProps) {
	return (
		<div className="flex flex-col gap-3 border-t border-border pt-3">
			<span className="text-xs font-medium text-muted-foreground">
				Settings
			</span>

			<div className="flex flex-col gap-1">
				<label
					htmlFor="remote-port"
					className="text-[10px] text-muted-foreground"
				>
					Port
				</label>
				<Input
					id="remote-port"
					type="number"
					min={1024}
					max={65535}
					value={portInput}
					onChange={(e) => onPortInputChange(e.target.value)}
					onBlur={onPortBlur}
					disabled={running}
					variant="panel"
					size="xs"
					className="font-mono"
				/>
			</div>
		</div>
	);
}

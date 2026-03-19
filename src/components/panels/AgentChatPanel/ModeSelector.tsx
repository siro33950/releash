import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { PermissionMode } from "@/types/session";

const MODES: { value: PermissionMode; label: string }[] = [
	{ value: "acceptEdits", label: "Code" },
	{ value: "default", label: "Ask" },
	{ value: "plan", label: "Plan" },
	{ value: "bypassPermissions", label: "Bypass" },
];

interface ModeSelectorProps {
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	disabled: boolean;
}

export function ModeSelector({
	mode,
	onModeChange,
	disabled,
}: ModeSelectorProps) {
	return (
		<div data-testid="mode-selector" className="flex gap-1 px-3 py-1.5">
			{MODES.map((m) => {
				const isActive = mode === m.value;
				return (
					<Button
						key={m.value}
						variant={isActive ? "default" : "ghost"}
						size="xs"
						disabled={disabled}
						data-active={isActive ? "true" : undefined}
						onClick={() => onModeChange(m.value)}
						className={cn(isActive && "pointer-events-none")}
					>
						{m.label}
					</Button>
				);
			})}
		</div>
	);
}

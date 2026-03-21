import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { PermissionMode } from "@/types/session";

export const MODES: { value: PermissionMode; label: string }[] = [
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
	const currentLabel = MODES.find((m) => m.value === mode)?.label ?? "Code";

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="xs"
					disabled={disabled}
					data-testid="mode-selector-trigger"
					className="gap-1"
				>
					{currentLabel}
					<ChevronDown className="size-3" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent side="top" align="start">
				<DropdownMenuRadioGroup
					value={mode}
					onValueChange={(v) => onModeChange(v as PermissionMode)}
				>
					{MODES.map((m) => (
						<DropdownMenuRadioItem key={m.value} value={m.value}>
							{m.label}
						</DropdownMenuRadioItem>
					))}
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

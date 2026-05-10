import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { BackendInfo } from "@/types/session";

interface BackendSelectorProps {
	backends: BackendInfo[];
	selectedBackendId: string | null;
	onBackendChange: (backendId: string | null) => void;
	disabled: boolean;
}

export function BackendSelector({
	backends,
	selectedBackendId,
	onBackendChange,
	disabled,
}: BackendSelectorProps) {
	const availableBackends = backends.filter((b) => b.available);
	if (availableBackends.length <= 1) return null;

	const currentLabel =
		availableBackends.find((b) => b.id === selectedBackendId)?.name ??
		availableBackends[0]?.name ??
		"Backend";

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="xs"
					disabled={disabled}
					data-testid="backend-selector-trigger"
					className="gap-1 text-muted-foreground"
				>
					{currentLabel}
					<ChevronDown className="size-3" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent side="bottom" align="start">
				<DropdownMenuRadioGroup
					value={
						availableBackends.some((b) => b.id === selectedBackendId)
							? (selectedBackendId ?? "")
							: ""
					}
					onValueChange={(v) => onBackendChange(v || null)}
				>
					{availableBackends.map((b) => (
						<DropdownMenuRadioItem key={b.id} value={b.id}>
							{b.name}
						</DropdownMenuRadioItem>
					))}
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

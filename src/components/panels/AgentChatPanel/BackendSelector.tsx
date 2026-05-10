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
	if (backends.length <= 1) return null;

	const currentLabel =
		backends.find((b) => b.id === selectedBackendId)?.name ??
		backends[0]?.name ??
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
					value={selectedBackendId ?? ""}
					onValueChange={(v) => onBackendChange(v || null)}
				>
					{backends
						.filter((b) => b.available)
						.map((b) => (
							<DropdownMenuRadioItem key={b.id} value={b.id}>
								{b.name}
							</DropdownMenuRadioItem>
						))}
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

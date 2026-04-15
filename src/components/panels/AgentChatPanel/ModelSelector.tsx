import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { ModelInfo } from "@/types/session";

interface ModelSelectorProps {
	models: ModelInfo[];
	currentModelId: string | null;
	onModelChange: (modelId: string | null) => void;
	disabled: boolean;
}

const AUTO_VALUE = "__auto__";

export function ModelSelector({
	models,
	currentModelId,
	onModelChange,
	disabled,
}: ModelSelectorProps) {
	const currentLabel =
		models.find((m) => m.value === currentModelId)?.displayName ?? "Auto";

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="xs"
					disabled={disabled || models.length === 0}
					data-testid="model-selector-trigger"
					className="gap-1"
				>
					{currentLabel}
					<ChevronDown className="size-3" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent side="top" align="start">
				<DropdownMenuRadioGroup
					value={currentModelId ?? AUTO_VALUE}
					onValueChange={(v) => onModelChange(v === AUTO_VALUE ? null : v)}
				>
					<DropdownMenuRadioItem value={AUTO_VALUE}>Auto</DropdownMenuRadioItem>
					{models.map((m) => (
						<DropdownMenuRadioItem key={m.value} value={m.value}>
							{m.displayName}
						</DropdownMenuRadioItem>
					))}
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

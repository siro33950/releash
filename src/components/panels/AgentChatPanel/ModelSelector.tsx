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
	currentModelId: string;
	onModelChange: (modelId: string) => void;
	disabled: boolean;
}

export function ModelSelector({
	models,
	currentModelId,
	onModelChange,
	disabled,
}: ModelSelectorProps) {
	const currentLabel =
		models.find((m) => m.value === currentModelId)?.value ?? currentModelId;

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="xs"
					disabled={disabled}
					data-testid="model-selector-trigger"
					className="gap-1"
				>
					{currentLabel}
					<ChevronDown className="size-3" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent side="top" align="start">
				<DropdownMenuRadioGroup
					value={currentModelId}
					onValueChange={(v) => onModelChange(v)}
				>
					{models.map((m) => (
						<DropdownMenuRadioItem key={m.value} value={m.value}>
							{m.value}
						</DropdownMenuRadioItem>
					))}
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

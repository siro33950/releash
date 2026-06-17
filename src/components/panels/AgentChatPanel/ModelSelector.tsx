import claudeIcon from "@lobehub/icons-static-svg/icons/claude-color.svg";
import codexIcon from "@lobehub/icons-static-svg/icons/codex-color.svg";
import { Check, ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
	getModelInfoBackend,
	getModelInfoDisplayName,
	getModelInfoId,
	type ModelInfo,
	normalizeModelSelectionId,
} from "@/types/session";

const PROVIDER_ICONS: Record<string, string | undefined> = {
	claude: claudeIcon,
	codex: codexIcon,
};

function ProviderIcon({ backend }: { backend: string }) {
	const src = PROVIDER_ICONS[backend];
	if (!src) return null;
	return <img src={src} alt="" aria-hidden className="size-3.5" />;
}

interface ModelSelectorProps {
	models: ModelInfo[];
	currentModelId: string;
	currentBackendId?: string | null;
	canChangeBackend?: boolean;
	onModelChange: (modelId: string) => void;
	disabled: boolean;
}

export function ModelSelector({
	models,
	currentModelId,
	currentBackendId = null,
	canChangeBackend = true,
	onModelChange,
	disabled,
}: ModelSelectorProps) {
	const selectedModelId = normalizeModelSelectionId(models, currentModelId);
	const currentModel = models.find(
		(model) => getModelInfoId(model) === selectedModelId,
	);
	const currentLabel = currentModel
		? getModelInfoDisplayName(currentModel)
		: currentModelId;

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
					{currentModel ? (
						<ProviderIcon backend={getModelInfoBackend(currentModel)} />
					) : null}
					{currentLabel}
					<ChevronDown className="size-3" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent side="top" align="start" className="min-w-44">
				{models.map((model) => {
					const id = getModelInfoId(model);
					const backend = getModelInfoBackend(model);
					const isSelected = id === selectedModelId;
					const itemDisabled =
						!canChangeBackend && backend !== "" && backend !== currentBackendId;
					return (
						<DropdownMenuItem
							key={id}
							disabled={itemDisabled}
							onSelect={() => onModelChange(id)}
							className="gap-2"
						>
							<span className="flex size-4 items-center justify-center">
								{isSelected ? <Check className="size-3.5" /> : null}
							</span>
							<ProviderIcon backend={backend} />
							<span>{getModelInfoDisplayName(model)}</span>
						</DropdownMenuItem>
					);
				})}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

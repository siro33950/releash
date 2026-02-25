import { ArrowRight, GitBranch } from "lucide-react";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";

interface BranchSelectorProps {
	branchName: string | null;
	baseBranch: string | null;
	localBranches: string[];
	onBaseChange: (base: string) => void;
}

export function BranchSelector({
	branchName,
	baseBranch,
	localBranches,
	onBaseChange,
}: BranchSelectorProps) {
	if (!branchName) return null;

	return (
		<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
			<GitBranch className="size-3.5 shrink-0" />
			<span className="font-mono whitespace-nowrap">{branchName}</span>
			{localBranches.length > 0 && (
				<>
					<ArrowRight className="size-3 shrink-0" />
					<Select value={baseBranch} onValueChange={onBaseChange}>
						<SelectTrigger
							size="sm"
							className="h-5 min-w-0 max-w-[120px] border-none bg-transparent shadow-none px-1 text-xs font-mono"
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{localBranches.map((name) => (
								<SelectItem key={name} value={name}>
									{name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</>
			)}
		</div>
	);
}

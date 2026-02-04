import { Files, GitBranch, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

interface ActivityBarItem {
	id: string;
	icon: React.ReactNode;
	title: string;
}

const items: ActivityBarItem[] = [
	{ id: "explorer", icon: <Files className="size-5" />, title: "Explorer" },
	{ id: "search", icon: <Search className="size-5" />, title: "Search" },
	{
		id: "git",
		icon: <GitBranch className="size-5" />,
		title: "Source Control",
	},
];

interface ActivityBarProps {
	className?: string;
	activeItem?: string;
	onItemClick?: (id: string) => void;
}

export function ActivityBar({
	className,
	activeItem,
	onItemClick,
}: ActivityBarProps) {
	return (
		<div
			className={cn(
				"flex flex-col items-center w-12 py-1",
				"bg-sidebar border-r border-sidebar-border",
				className,
			)}
		>
			{items.map((item) => (
				<Tooltip key={item.id}>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className={cn(
								"w-12 h-12 rounded-none",
								activeItem === item.id
									? "border-l-2 border-sidebar-primary text-sidebar-foreground bg-sidebar-accent"
									: "text-muted-foreground hover:text-sidebar-foreground",
							)}
							onClick={() => onItemClick?.(item.id)}
						>
							{item.icon}
						</Button>
					</TooltipTrigger>
					<TooltipContent side="right">{item.title}</TooltipContent>
				</Tooltip>
			))}
		</div>
	);
}

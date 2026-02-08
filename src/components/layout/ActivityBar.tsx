import { Files, GitBranch, Globe, Search, Settings } from "lucide-react";
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
	{
		id: "remote",
		icon: <Globe className="size-5" />,
		title: "Remote",
	},
];

interface ActivityBarProps {
	className?: string;
	activeItem?: string;
	onItemClick?: (id: string) => void;
}

function ActivityBarButton({
	item,
	isActive,
	onClick,
}: {
	item: ActivityBarItem;
	isActive: boolean;
	onClick: () => void;
}) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					aria-label={item.title}
					className={cn(
						"w-12 h-12 rounded-none",
						isActive
							? "border-l-2 border-sidebar-primary text-sidebar-foreground bg-sidebar-accent"
							: "text-muted-foreground hover:text-sidebar-foreground",
					)}
					onClick={onClick}
				>
					{item.icon}
				</Button>
			</TooltipTrigger>
			<TooltipContent side="right">{item.title}</TooltipContent>
		</Tooltip>
	);
}

const settingsItem: ActivityBarItem = {
	id: "settings",
	icon: <Settings className="size-5" />,
	title: "Settings",
};

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
				<ActivityBarButton
					key={item.id}
					item={item}
					isActive={activeItem === item.id}
					onClick={() => onItemClick?.(item.id)}
				/>
			))}
			<div className="mt-auto">
				<ActivityBarButton
					item={settingsItem}
					isActive={activeItem === "settings"}
					onClick={() => onItemClick?.("settings")}
				/>
			</div>
		</div>
	);
}

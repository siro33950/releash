import {
	Files,
	GitBranch,
	GitPullRequest,
	LayoutGrid,
	Search,
	Settings,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export interface ActivityBarItem {
	id: string;
	icon: React.ReactNode;
	title: string;
}

const defaultItems: ActivityBarItem[] = [
	{
		id: "git",
		icon: <GitBranch className="size-5" />,
		title: "Source Control",
	},
	{ id: "explorer", icon: <Files className="size-5" />, title: "Explorer" },
	{ id: "search", icon: <Search className="size-5" />, title: "Search" },
	{
		id: "pr",
		icon: <GitPullRequest className="size-5" />,
		title: "Pull Request",
	},
];

const defaultBottomItems: ActivityBarItem[] = [
	{
		id: "settings",
		icon: <Settings className="size-5" />,
		title: "Settings",
	},
];

interface ActivityBarProps {
	className?: string;
	activeItem?: string;
	onItemClick?: (id: string) => void;
	onGoHome?: () => void;
	items?: ActivityBarItem[];
	bottomItems?: ActivityBarItem[];
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

const homeItem: ActivityBarItem = {
	id: "home",
	icon: <LayoutGrid className="size-5" />,
	title: "Workspace Manager",
};

export function ActivityBar({
	className,
	activeItem,
	onItemClick,
	onGoHome,
	items = defaultItems,
	bottomItems = defaultBottomItems,
}: ActivityBarProps) {
	return (
		<div
			className={cn(
				"flex flex-col items-center w-12 py-1",
				"bg-activity-bar border-r border-activity-bar-border",
				className,
			)}
		>
			{onGoHome && (
				<>
					<ActivityBarButton
						item={homeItem}
						isActive={false}
						onClick={onGoHome}
					/>
					<div className="w-8 border-b border-activity-bar-border my-1" />
				</>
			)}
			{items.map((item) => (
				<ActivityBarButton
					key={item.id}
					item={item}
					isActive={activeItem === item.id}
					onClick={() => onItemClick?.(item.id)}
				/>
			))}
			{bottomItems.length > 0 && (
				<div className="mt-auto">
					{bottomItems.map((item) => (
						<ActivityBarButton
							key={item.id}
							item={item}
							isActive={activeItem === item.id}
							onClick={() => onItemClick?.(item.id)}
						/>
					))}
				</div>
			)}
		</div>
	);
}

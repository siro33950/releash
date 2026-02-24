import { X } from "lucide-react";
import type {
	ComponentPropsWithoutRef,
	KeyboardEvent,
	MouseEvent,
	ReactNode,
} from "react";
import { cn } from "@/lib/utils";

interface TabBarContainerProps {
	children: ReactNode;
	className?: string;
	ariaLabel?: string;
}

export function TabBarContainer({
	children,
	className,
	ariaLabel,
}: TabBarContainerProps) {
	return (
		<div
			role="tablist"
			aria-orientation="horizontal"
			aria-label={ariaLabel}
			className={cn(
				"flex items-center h-[34px] bg-sidebar border-b border-border shrink-0 overflow-x-auto",
				className,
			)}
		>
			{children}
		</div>
	);
}

type TabBarItemBaseProps = {
	isActive: boolean;
	onClick: () => void;
	onClose?: (e: MouseEvent) => void;
	closeLabel?: string;
	children: ReactNode;
	className?: string;
	id?: string;
	ariaControls?: string;
	ariaLabel?: string;
	onKeyDown?: (e: KeyboardEvent) => void;
};

type TabBarItemProps = TabBarItemBaseProps &
	Omit<ComponentPropsWithoutRef<"div">, keyof TabBarItemBaseProps>;

export function TabBarItem({
	isActive,
	onClick,
	onClose,
	closeLabel,
	children,
	className,
	id,
	ariaControls,
	ariaLabel,
	onKeyDown,
	...rest
}: TabBarItemProps) {
	const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
		if (e.key === "Enter" || e.key === " ") {
			e.preventDefault();
			onClick();
		}
		onKeyDown?.(e);
	};

	return (
		<div
			id={id}
			role="tab"
			aria-selected={isActive}
			aria-controls={ariaControls}
			aria-label={ariaLabel}
			tabIndex={isActive ? 0 : -1}
			className={cn(
				"group flex items-center gap-2 h-full px-3 text-sm border-r border-border cursor-pointer transition-colors shrink-0",
				isActive
					? "bg-background text-foreground"
					: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
				className,
			)}
			onClick={onClick}
			onKeyDown={handleKeyDown}
			{...rest}
		>
			{children}
			{onClose && (
				<button
					type="button"
					onClick={(e) => {
						e.stopPropagation();
						onClose(e);
					}}
					className={cn(
						"p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0",
						isActive
							? "opacity-100"
							: "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
					)}
					aria-label={closeLabel}
				>
					<X className="size-3.5" />
				</button>
			)}
		</div>
	);
}

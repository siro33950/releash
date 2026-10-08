const TOOLTIP_OFFSET_PX = 12;
const TOOLTIP_Z_INDEX = "12";

export interface TerminalLinkTooltip {
	hover(event: MouseEvent, url: string): void;
	leave(): void;
	dispose(): void;
}

export function createTerminalLinkTooltip(
	target: HTMLElement,
): TerminalLinkTooltip {
	let tooltip: HTMLDivElement | null = null;
	let currentTarget: HTMLElement | null = target;

	const leave = (): void => {
		tooltip?.remove();
		tooltip = null;
	};

	return {
		hover(event, url) {
			if (!currentTarget) return;
			if (!tooltip) {
				tooltip = document.createElement("div");
				tooltip.className = "xterm-hover";
				tooltip.role = "tooltip";
				Object.assign(tooltip.style, {
					position: "absolute",
					zIndex: TOOLTIP_Z_INDEX,
					boxSizing: "border-box",
					width: "max-content",
					maxWidth: `calc(100% - ${TOOLTIP_OFFSET_PX * 2}px)`,
					overflowWrap: "anywhere",
					borderRadius: "0.375rem",
					padding: "0.375rem 0.75rem",
					background: "var(--foreground)",
					color: "var(--background)",
					fontSize: "0.75rem",
					lineHeight: "1rem",
					boxShadow: "0 4px 6px -1px rgb(0 0 0 / 0.1)",
				});
				currentTarget.append(tooltip);
			}

			const bounds = currentTarget.getBoundingClientRect();
			tooltip.textContent = url;
			const tooltipBounds = tooltip.getBoundingClientRect();
			const pointerLeft = event.clientX - bounds.left;
			const pointerTop = event.clientY - bounds.top;
			const maxLeft = Math.max(
				TOOLTIP_OFFSET_PX,
				bounds.width - tooltipBounds.width - TOOLTIP_OFFSET_PX,
			);
			const maxTop = Math.max(
				TOOLTIP_OFFSET_PX,
				bounds.height - tooltipBounds.height - TOOLTIP_OFFSET_PX,
			);
			const left = Math.min(
				Math.max(pointerLeft + TOOLTIP_OFFSET_PX, TOOLTIP_OFFSET_PX),
				maxLeft,
			);
			const preferredTop = pointerTop + TOOLTIP_OFFSET_PX;
			const top = Math.min(
				Math.max(
					preferredTop <= maxTop
						? preferredTop
						: pointerTop - tooltipBounds.height - TOOLTIP_OFFSET_PX,
					TOOLTIP_OFFSET_PX,
				),
				maxTop,
			);
			tooltip.style.left = `${left}px`;
			tooltip.style.top = `${top}px`;
		},
		leave,
		dispose() {
			leave();
			currentTarget = null;
		},
	};
}

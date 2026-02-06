import { ScrollArea } from "@/components/ui/scroll-area";
import type { AppSettings, Theme } from "@/types/settings";

export interface SettingsPanelProps {
	settings: AppSettings;
	onThemeChange: (theme: Theme) => void;
	onFontSizeChange: (size: number) => void;
}

export function SettingsPanel({
	settings,
	onThemeChange,
	onFontSizeChange,
}: SettingsPanelProps) {
	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					Settings
				</span>
			</div>

			<ScrollArea className="flex-1 min-h-0">
				<div className="px-3 py-3 flex flex-col gap-4">
					<div className="flex flex-col gap-1.5">
						<label
							htmlFor="theme-select"
							className="text-xs font-medium text-muted-foreground"
						>
							Theme
						</label>
						<select
							id="theme-select"
							value={settings.theme}
							onChange={(e) => onThemeChange(e.target.value as Theme)}
							className="w-full bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
						>
							<option value="dark">Dark</option>
							<option value="light">Light</option>
						</select>
					</div>

					<div className="flex flex-col gap-1.5">
						<label
							htmlFor="font-size-slider"
							className="text-xs font-medium text-muted-foreground"
						>
							Font Size: {settings.fontSize}px
						</label>
						<input
							id="font-size-slider"
							type="range"
							min={12}
							max={24}
							step={1}
							value={settings.fontSize}
							onChange={(e) => onFontSizeChange(Number(e.target.value))}
							className="w-full accent-primary"
						/>
						<div className="flex justify-between text-[10px] text-muted-foreground">
							<span>12px</span>
							<span>24px</span>
						</div>
					</div>
				</div>
			</ScrollArea>
		</div>
	);
}

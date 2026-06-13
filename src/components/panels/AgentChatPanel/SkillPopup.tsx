import type { AgentSkill } from "@/types/session";
import { AutocompletePopup } from "./AutocompletePopup";

interface SkillPopupProps {
	open: boolean;
	skills: AgentSkill[];
	selectedIndex: number;
	onSelect: (skill: AgentSkill) => void;
	onClose: () => void;
	anchorRef: React.RefObject<HTMLElement | null>;
}

export function SkillPopup({
	open,
	skills,
	selectedIndex,
	onSelect,
	onClose,
	anchorRef,
}: SkillPopupProps) {
	return (
		<AutocompletePopup
			open={open}
			items={skills}
			selectedIndex={selectedIndex}
			onSelect={onSelect}
			onClose={onClose}
			anchorRef={anchorRef}
			getKey={(skill) => `${skill.scope}:${skill.name}`}
			renderItem={(skill) => (
				<>
					<span className="font-medium">${skill.name}</span>
					<span className="text-xs text-muted-foreground">
						{skill.description
							? `${skill.scope} - ${skill.description}`
							: skill.scope}
					</span>
				</>
			)}
			testId="skill-list"
			itemClassName="flex flex-col"
		/>
	);
}

import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	ReviewThreadHandoffProvider,
	useReviewThreadHandoff,
} from "./ReviewThreadHandoffContext";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	writeText: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mocks.invoke(...args),
}));

function HandoffButton() {
	const { canCopy, copyThreadForAgent, feedback } = useReviewThreadHandoff();
	return (
		<>
			<button
				type="button"
				disabled={!canCopy}
				onClick={() => copyThreadForAgent("thread-1")}
			>
				Copy for Agent
			</button>
			{feedback && (
				<div role={feedback.kind === "error" ? "alert" : "status"}>
					{feedback.message}
				</div>
			)}
		</>
	);
}

function renderHandoff() {
	return render(
		<ReviewThreadHandoffProvider worktreeName="feature/1599">
			<HandoffButton />
		</ReviewThreadHandoffProvider>,
	);
}

beforeEach(() => {
	vi.clearAllMocks();
	mocks.invoke.mockResolvedValue("review instruction");
	mocks.writeText.mockResolvedValue(undefined);
	Object.defineProperty(navigator, "clipboard", {
		configurable: true,
		value: { writeText: mocks.writeText },
	});
});

describe("ReviewThreadHandoffProvider", () => {
	it("active AgentSessionがなくてもRust生成instructionをclipboardへcopyする", async () => {
		renderHandoff();

		const button = screen.getByRole("button", { name: "Copy for Agent" });
		expect(button).toBeEnabled();
		fireEvent.click(button);

		expect(await screen.findByRole("status")).toHaveTextContent(
			"Agent instruction copied",
		);
		expect(mocks.invoke).toHaveBeenCalledWith("build_review_thread_handoff", {
			worktreeName: "feature/1599",
			threadId: "thread-1",
		});
		expect(mocks.writeText).toHaveBeenCalledWith("review instruction");
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"send_agent_message",
			expect.anything(),
		);
	});

	it("clipboard copy失敗を呼び出し元が表示できる", async () => {
		mocks.writeText.mockRejectedValueOnce(new Error("clipboard unavailable"));
		renderHandoff();

		fireEvent.click(screen.getByRole("button", { name: "Copy for Agent" }));

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Failed to copy Agent instruction: clipboard unavailable",
		);
	});

	it("handoff生成失敗はbackend文言だけを表示してclipboardを呼ばない", async () => {
		mocks.invoke.mockRejectedValueOnce({
			code: "REVIEW_HANDOFF_UNAVAILABLE",
			message: "Review handoff is unavailable. Try again.",
		});
		renderHandoff();

		fireEvent.click(screen.getByRole("button", { name: "Copy for Agent" }));

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Review handoff is unavailable. Try again.",
		);
		expect(mocks.writeText).not.toHaveBeenCalled();
	});
});

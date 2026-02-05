import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusBar } from "./StatusBar";

describe("StatusBar", () => {
	it("should display branch name with git icon", () => {
		render(<StatusBar branch="feat/test" />);

		expect(screen.getByText("feat/test")).toBeInTheDocument();
	});

	it("should display file info when language is provided", () => {
		render(
			<StatusBar
				branch="main"
				language="typescript"
				encoding="UTF-8"
				eol="LF"
			/>,
		);

		expect(screen.getByText("TypeScript")).toBeInTheDocument();
		expect(screen.getByText("UTF-8")).toBeInTheDocument();
		expect(screen.getByText("LF")).toBeInTheDocument();
	});

	it("should display CRLF when eol is CRLF", () => {
		render(<StatusBar language="javascript" encoding="UTF-8" eol="CRLF" />);

		expect(screen.getByText("CRLF")).toBeInTheDocument();
	});

	it("should hide file info when no file is open", () => {
		render(<StatusBar branch="main" />);

		expect(screen.queryByText("UTF-8")).not.toBeInTheDocument();
		expect(screen.queryByText("LF")).not.toBeInTheDocument();
	});

	it("should display language display name for known languages", () => {
		render(<StatusBar language="rust" encoding="UTF-8" eol="LF" />);

		expect(screen.getByText("Rust")).toBeInTheDocument();
	});

	it("should fall back to language ID for unknown languages", () => {
		render(<StatusBar language="unknown-lang" encoding="UTF-8" eol="LF" />);

		expect(screen.getByText("unknown-lang")).toBeInTheDocument();
	});
});

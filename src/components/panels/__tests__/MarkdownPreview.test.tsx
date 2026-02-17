import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownPreview } from "../MarkdownPreview";

describe("MarkdownPreview", () => {
	it("renders markdown as HTML", () => {
		render(<MarkdownPreview content="**bold text**" />);
		const strong = screen.getByText("bold text");
		expect(strong.tagName).toBe("STRONG");
	});

	it("renders GFM tables", () => {
		const table = `| Header | Value |
| ------ | ----- |
| A      | 1     |`;
		render(<MarkdownPreview content={table} />);
		expect(screen.getByText("Header")).toBeInTheDocument();
		expect(screen.getByText("Value")).toBeInTheDocument();
		expect(screen.getByText("A")).toBeInTheDocument();
	});

	it("has data-testid", () => {
		render(<MarkdownPreview content="hello" />);
		expect(screen.getByTestId("markdown-preview")).toBeInTheDocument();
	});

	it("renders headings", () => {
		render(<MarkdownPreview content="## Heading 2" />);
		const heading = screen.getByRole("heading", { level: 2 });
		expect(heading).toHaveTextContent("Heading 2");
	});

	it("renders links", () => {
		render(<MarkdownPreview content="[example](https://example.com)" />);
		const link = screen.getByRole("link");
		expect(link).toHaveAttribute("href", "https://example.com");
		expect(link).toHaveTextContent("example");
	});
});

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ImageDiffViewer } from "../ImageDiffViewer";

describe("ImageDiffViewer", () => {
	it("shows loading state", () => {
		render(<ImageDiffViewer originalUrl={null} modifiedUrl={null} loading />);
		expect(screen.getByText("Loading...")).toBeInTheDocument();
	});

	it("renders both images when URLs provided", () => {
		render(
			<ImageDiffViewer
				originalUrl="data:image/png;base64,AAA"
				modifiedUrl="data:image/png;base64,BBB"
				loading={false}
			/>,
		);
		const images = screen.getAllByRole("img");
		expect(images).toHaveLength(2);
		expect(images[0]).toHaveAttribute("src", "data:image/png;base64,AAA");
		expect(images[1]).toHaveAttribute("src", "data:image/png;base64,BBB");
	});

	it("shows 'No file' when originalUrl is null (new file)", () => {
		render(
			<ImageDiffViewer
				originalUrl={null}
				modifiedUrl="data:image/png;base64,BBB"
				loading={false}
			/>,
		);
		expect(screen.getByText("No file")).toBeInTheDocument();
		expect(screen.getAllByRole("img")).toHaveLength(1);
	});

	it("shows 'No file' when modifiedUrl is null (deleted file)", () => {
		render(
			<ImageDiffViewer
				originalUrl="data:image/png;base64,AAA"
				modifiedUrl={null}
				loading={false}
			/>,
		);
		expect(screen.getByText("No file")).toBeInTheDocument();
		expect(screen.getAllByRole("img")).toHaveLength(1);
	});

	it("shows labels Original and Modified", () => {
		render(
			<ImageDiffViewer
				originalUrl="data:image/png;base64,AAA"
				modifiedUrl="data:image/png;base64,BBB"
				loading={false}
			/>,
		);
		expect(screen.getByText("Original")).toBeInTheDocument();
		expect(screen.getByText("Modified")).toBeInTheDocument();
	});
});

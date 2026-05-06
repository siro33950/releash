You MUST include exactly one `<workflow_output>` block in your response.

Format:
```
<workflow_output type="review-verdict">
{
  "verdict": "LGTM" or "NEEDS_FIX",
  "findings": [
    { "severity": "error" | "warning" | "info", "message": "description" }
  ]
}
</workflow_output>
```

Rules:
- `verdict` is required: "LGTM" (no issues) or "NEEDS_FIX" (issues found)
- `findings` is required when verdict is "NEEDS_FIX" (at least one entry)
- `findings` may be empty or omitted when verdict is "LGTM"
- Each finding must have `severity` and `message`

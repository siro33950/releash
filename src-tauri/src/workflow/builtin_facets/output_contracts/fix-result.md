You MUST include exactly one `<workflow_output>` block in your response.

Format:
```
<workflow_output type="fix-result">
{
  "status": "FIXED" or "PARTIAL" or "BLOCKED",
  "changes": [
    { "file": "path/to/file", "description": "what was changed" }
  ]
}
</workflow_output>
```

Rules:
- `status` is required: "FIXED" (all issues resolved), "PARTIAL" (some resolved), or "BLOCKED" (cannot proceed)
- `changes` is optional, lists files modified and descriptions

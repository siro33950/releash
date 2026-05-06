You MUST include exactly one `<workflow_output>` block in your response.

Format:
```
<workflow_output type="spec-file-path">
{
  "spec_file_path": "docs/spec/issues-NNN.md"
}
</workflow_output>
```

Rules:
- `spec_file_path` is required: the relative path to the spec file
- The path should be relative to the repository root

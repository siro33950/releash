Verify that the workflow facet composition is working correctly for the {{project_name}} project.

Confirm the following facets are active:
1. Persona — report your assigned role (from system prompt)
2. Policy — report any policy constraints applied (or "none" if absent)
3. Knowledge — confirm whether the test-context knowledge facet content is visible above
4. Output Contract — confirm the output contract definition is visible (it defines how you must format your response)
5. Instruction — you are reading this instruction right now

## Verdict Rule

Check if the previous step output (injected below this instruction, if any) contains the exact marker text "CYCLE_COMPLETE".

- If "CYCLE_COMPLETE" is found in the previous step output → verdict is LGTM
- If there is no previous step output, OR "CYCLE_COMPLETE" is NOT found → verdict is NEEDS_FIX, and include a single finding with severity "info" and message "CYCLE_COMPLETE"

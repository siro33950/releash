---
type: instruction
key: implementation-fix-policy
description: Draft an approval policy before applying code review fixes
---

# Implementation Fix Policy Approval

Review all provided code review outputs, including LGTM and NEEDS_FIX results.

Draft the exact policy that should be used by the following `fix` step. Focus on:

- which code review findings must be fixed
- which findings should be left unchanged and why
- constraints the fix step must respect
- tests or checks that should be run after the fix

Do not edit files in this step. The user may refine the policy in chat before approval.

When the policy is ready, output only:

<workflow_output type="approved-fix-policy">
{
  "policy": "<approved implementation fix policy>",
  "review_step": "code_review_parallel"
}
</workflow_output>

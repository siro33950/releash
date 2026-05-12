---
type: instruction
key: plan-fix-policy
description: Draft an approval policy before applying plan review fixes
---

# Plan Fix Policy Approval

Review all provided plan review outputs, including LGTM and NEEDS_FIX results.

Draft the exact policy that should be used by the following `plan_fix` step. Focus on:

- which findings must be fixed
- which findings should be left unchanged and why
- constraints the fix step must respect

Do not edit files in this step. The user may refine the policy in chat before approval.

When the policy is ready, output only:

<workflow_output type="approved-fix-policy">
{
  "policy": "<approved plan fix policy>",
  "review_step": "plan_review_parallel"
}
</workflow_output>

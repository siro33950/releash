---
type: output_contract
key: approved-fix-policy
description: Approved policy for a subsequent fix step
---

Return exactly one workflow output block:

<workflow_output type="approved-fix-policy">
{
  "policy": "Concrete fix policy approved by the user. Include only information needed for the fix step.",
  "review_step": "The review or aggregate step name this policy responds to."
}
</workflow_output>

`policy` must be non-empty and no larger than 65536 UTF-8 bytes. `review_step` must name the review source: `plan_review_parallel` for Plan fix policy or `code_review_parallel` for implementation fix policy.

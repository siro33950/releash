local r = require("releash")
local result = r.schema.object{
  name = "result",
  properties = {
    passed = r.schema.boolean(),
    clean = r.schema.boolean(),
    skipped = r.schema.boolean(),
    ["legacy flag"] = r.schema.boolean(),
    details = r.schema.object{
      properties = { passed = r.schema.boolean(), text = r.schema.string{}, optional = r.schema.boolean() },
      required = { "passed", "text" },
    },
  },
  required = { "passed", "clean", "skipped", "legacy flag" },
}
local judge = r.command{ name = "judge", command = "judge", artifact = result }
local done = r.command{ name = "done", command = "done" }
local fix = r.command{ name = "fix", command = "fix" }
return r.workflow{
  name = "predicate-routing", description = "Nested boolean routing",
  main = r.sequence{ children = {
    r.child{ node = judge, rules = {
      r.when{ on = r.all{ judge.passed, r.any{ judge.clean, judge.skipped } }, on_true = done, next = fix },
    } },
    r.child{ node = done, rules = {} },
    r.child{ node = fix, rules = {} },
  } },
}

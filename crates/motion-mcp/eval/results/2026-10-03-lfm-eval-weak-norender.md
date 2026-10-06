# motion-mcp eval: lfm-eval on the weak profile

- date 2026-10-03, check only (--no-render): story quality, no render, no QA
- endpoint http://localhost:1234/v1, temperature 0.2, seed 7, system prompt generic, max 6 turns
- tools: make_video, revise_video, get_video, find_assets

| brief | success | calls | fix retries | tokens in/out | wall s | video s | note |
|---|---|---:|---:|---:|---:|---:|---|
| ai_agent_parts | yes | 1 | 0 | 1229/751 | 29.1 | - |  |
| ai_training_power | yes | 1 | 0 | 1231/424 | 20.0 | - |  |
| ai_proofreader | no | 5 | 3 | 7979/3577 | 159.5 | - | brief: 7 beats, wanted 2-6; same call repeated 2x |
| sci_photosynthesis | no | 4 | 3 | 6000/2852 | 117.9 | - | brief: missing: sunlight; same call repeated 2x |
| sci_ocean_zones | yes | 3 | 2 | 4665/3147 | 124.9 | - |  |
| sci_atmosphere | no | 0 | 0 | 1224/890 | 36.4 | - | no tool call: # The Layers of Earth's Atmosphere (From Ground Up)  ## 1. Tropospher… |
| sci_water_states | yes | 1 | 0 | 1223/1148 | 50.9 | - |  |
| money_compound | yes | 1 | 0 | 1238/519 | 29.1 | - |  |
| money_coffee_inflation | no | 2 | 0 | 2502/666 | 30.4 | - | brief: 7 beats, wanted 2-6 |
| money_budget_rule | yes | 1 | 0 | 1221/764 | 29.8 | - |  |
| hist_pyramid | yes | 1 | 0 | 1231/1088 | 42.1 | - |  |
| hist_printing_press | yes | 1 | 0 | 1227/576 | 23.2 | - |  |
| hist_moon_landing | yes | 4 | 3 | 6108/2122 | 86.7 | - |  |
| sport_marathon | yes | 2 | 0 | 2506/1010 | 42.2 | - |  |
| sport_team_sizes | no | 6 | 6 | 9680/5090 | 207.4 | - | needs_fix: story: invalid type: null, expected struct LiteStory; same call repeated 4x; max turns |
| sport_sprint_training | no | 0 | 0 | 1221/581 | 23.1 | - | no tool call: empty answer (finish_reason stop) |
| psych_habit_loop | yes | 6 | 5 | 10484/3569 | 150.2 | - |  |
| psych_sleep | no | 5 | 5 | 10380/7746 | 322.6 | - | needs_fix: story: invalid type: null, expected struct LiteStory; same call repeated 4x; max turns |
| vague_space | no | 0 | 0 | 1207/503 | 18.5 | - | no tool call: empty answer (finish_reason stop) |
| vague_monday | yes | 4 | 3 | 5904/2334 | 84.8 | - |  |

## Summary against the plan targets (section 1, weak profile)

| metric | result | target | |
|---|---|---|---|
| success | 12/20 = 60 % | >= 85 % | MISS |
| tool calls per brief | 2.40 avg, 2.17 on the successful ones | <= 1.5 | MISS |
| tool definitions | 3582 chars, 1170 tokens measured on the model, first request 1229 prompt tokens | <= 1200 tokens | PASS |
| tool reply size | 49 tokens avg, 113 max (chars / 4, 48 replies) | <= 200 tokens | PASS |

- fix retries 30, bad arguments 0, repeated identical calls 19, tokens in/out 78460/39357, average wall 81.4 s per brief

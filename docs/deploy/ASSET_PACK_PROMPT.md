# Picture prompt (shown to visitors, copyable)

The pictures page shows the text between the two `---` lines with a Copy button. It must read as plain
art direction and say nothing about how the app uses the pictures (`PREVIEW_SITE_PLAN.md` §3): no engine
name, ids, file formats beyond PNG/JPG, roles, matching or cut-out method. Before showing it, the app fills
the `{{…}}` fields. Keep it in step with the upload rules in `PREVIEW_SITE_PLAN.md` §9.

---
Please make a set of pictures for my short video. Make each picture listed below, check every one, and
give me all of them in one zip file. You need an image tool you can use from here. If you have none, tell
me, and write one prompt per picture that I can paste into an image generator myself.

PICTURES TO MAKE
{{PICTURE_LIST}}

STYLE FOR THE WHOLE SET
{{STYLE}}
Use the same medium for every picture, with the same light direction, camera height, level of detail and
colours, so the pictures look like one set.

OBJECTS AND PEOPLE
- One subject per picture, centred. The whole subject is inside the frame with clear space on every side;
  nothing touches or crosses the edge.
- Background: one perfectly flat, solid colour from edge to edge: {{GROUND}}. No gradient, floor, horizon,
  table top, cast shadow, reflection or vignette.
- That colour must not appear anywhere on the subject: no tint, no reflection of it, and no rainbow or
  iridescent shine on glass or metal.
- If a subject is itself {{GROUND_CONFLICT}}, make that one picture on solid {{GROUND_FALLBACK}} instead.
- No text, letters, numbers, logos, watermarks, signatures, borders or frames in the picture.
- People: the whole person, or head to waist, with the head fully inside the frame. Do not show real or
  recognisable people unless you have the right to.
- Objects square; people portrait (3:4). At least 1024 pixels on the short side. PNG.
- If your image tool can make a truly transparent background, you may use that instead of the solid
  colour.

BACKGROUND SCENES (only if the list asks for one)
- A full scene, portrait (9:16 or 3:4), at least 1080 pixels wide, no text, with a calm area. No solid
  colour for these.

CHECK EVERY PICTURE BEFORE YOU KEEP IT
Look at each picture yourself and make it again if any answer is "no":
1. Is the whole subject inside the frame, with space on every side?
2. Is the background one flat colour edge to edge, with no shadow, floor or gradient?
3. Is that colour absent from the subject?
4. Is it free of text, numbers and logos?
5. Does it match the style and the other pictures?
6. Would a stranger name it the way I did in the list?
Keep the best version. After 3 tries, keep the best one and tell me what is wrong with it.

NAMES AND THE ZIP
- Name each file by what it shows, in plain words, as in the list: "piggy bank.png", "coin stack.png".
- Put background scenes in a folder called "backgrounds".
- Zip everything into one file of at most {{MAX_ZIP_MB}} MB with at most {{MAX_PICTURES}} pictures, and
  tell me where it is.
- Then give me a short table: file, how many tries it took, and anything you are unsure about.
---

## Fields the app fills

| field | value |
|---|---|
| `{{PICTURE_LIST}}` | one line per thing to draw, in plain words, from the visitor's storyboard: `- a piggy bank (scene 3)`. Without a storyboard: `- Make 8 to 20 pictures of the objects, people and places my videos will need, and list them for me first.` |
| `{{STYLE}}` | the style the visitor picked, e.g. `soft clay 3D props, warm studio light, rounded shapes, matte finish` (presets written as plain art direction) |
| `{{GROUND}}` | `magenta #FF00FF` (default), `green #00FF00` or `white #FFFFFF`, as the visitor picked |
| `{{GROUND_CONFLICT}}` | for magenta: `pink, purple or magenta`; for green: `green`; for white: `white, cream or very light grey` |
| `{{GROUND_FALLBACK}}` | for magenta: `green #00FF00`; for green and white: `magenta #FF00FF` |
| `{{MAX_ZIP_MB}}`, `{{MAX_PICTURES}}` | the upload limits (200, 60) |

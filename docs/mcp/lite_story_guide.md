# Lite story

Call make_video once: {"story": {"title": "...", "beats": [...]}, "style": "auto"}. 2-12 beats, one idea each. Brand colours: add "brand": {"primary": "#0A84FF"}.

## A beat
- say (required): what the narrator says, 8-30 words. All beats are read as ONE continuous script: each line carries on from the last ("But...", "So..."), never restarts the topic.
- show: on-screen title, 6 words or fewer; name the topic, not the answer. Left out, the engine uses keyword, else the first words of say.
- keyword: one word shown large and faint behind the beat. energy: calm | building | impact.

## One structure per beat
Pick at most one; with several, the first in this list wins.
- layers {names: [top..bottom], focus}: zones, floors, stages; repeat the same names in each beat about them
- list [3-6 words]
- compare {a, b, how: separate | grow | compress | replace}
- change {what, from, to}
- number "$381B" + meaning "cash pile" (+ picture)
- picture "piggy_bank" (+ picture2): a concrete lowercase noun. No match = shown as text; find_assets lists matches. Own image: "file:folder/name.jpg".
- none: the title carries the beat.

Use different structures in neighbouring beats. End on the takeaway. Later changes: revise_video(job, changes), never a new story.

Numbers on screen: only for a countdown or steps in order. Open each item's say with its rank ("Number three: ...", "Step 1: ...", "First, ..."); beats without one get no number.

## Examples
Number: {"say": "Warren Buffett is sitting on three hundred eighty one billion dollars in cash, more than ever before.", "show": "Buffett's cash pile", "number": "$381B", "meaning": "cash reserves", "picture": "money_bag"}

List: {"say": "Five tools now run the whole workflow, from the first draft to the final review.", "show": "The new toolkit", "list": ["robot", "microchip", "brain_circuit", "chat_bubbles", "ai_sparkles"]}

Layers: {"say": "Below the thermocline the water turns cold and dark, and almost nothing lives there.", "layers": {"names": ["sunlit", "twilight", "midnight"], "focus": "twilight"}}

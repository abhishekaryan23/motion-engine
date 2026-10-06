# assets/inbox

The default asset root of the MCP server (`motion-mcp`, Sprint 0.21): put a
folder of your own images here and pass its name as `make_video(…, assets:
"<folder>")`, or reference single files per beat as `"picture":
"file:<folder>/<image>"`. Images never travel through the model; the engine
reads, measures and treats them. Everything in this folder except this README
is gitignored.

Folder convention:

```
assets/inbox/my_trip/
  beat1.jpg            # picture for beat 1 (role guessed)
  beat2_person.png     # explicit role: person | object | place
  beat3_object.png
  background.jpg       # environment for every beat without its own
```

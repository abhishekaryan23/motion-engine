// (0.14) On-device subject matting with Apple Vision (macOS 14+): lifts the
// foreground subject(s) of a photo into an RGBA PNG. No network, no model
// download. Used by `motion-engine matte` (compiled once into target/).
//
//   matte_vision <in-image> <out.png> [--crop]
//
// Exit codes: 0 ok, 1 error, 2 usage, 3 no subject found.
import CoreImage
import Foundation
import Vision

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write("usage: matte_vision <in-image> <out.png> [--crop]\n".data(using: .utf8)!)
    exit(2)
}
let inURL = URL(fileURLWithPath: args[1])
let outURL = URL(fileURLWithPath: args[2])
let crop = args.contains("--crop")

func fail(_ msg: String, _ code: Int32 = 1) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(code)
}

guard let image = CIImage(contentsOf: inURL, options: [.applyOrientationProperty: true]) else {
    fail("cannot read \(inURL.path)")
}
let handler = VNImageRequestHandler(ciImage: image)
let request = VNGenerateForegroundInstanceMaskRequest()
do { try handler.perform([request]) } catch { fail("vision: \(error)") }
guard let observation = request.results?.first, !observation.allInstances.isEmpty else {
    fail("no subject found", 3)
}
do {
    let buffer = try observation.generateMaskedImage(
        ofInstances: observation.allInstances, from: handler, croppedToInstancesExtent: crop)
    let masked = CIImage(cvPixelBuffer: buffer)
    guard let srgb = CGColorSpace(name: CGColorSpace.sRGB) else { fail("no sRGB colour space") }
    try CIContext().writePNGRepresentation(of: masked, to: outURL, format: .RGBA8, colorSpace: srgb)
} catch {
    fail("mask: \(error)")
}

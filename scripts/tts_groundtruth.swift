// (0.20 A4) Ground-truth speech fixtures for the word-timing benchmark
// (crates/motion-voice/tests/timing_benchmark.rs). macOS 13+ speech synthesis
// writes a narration script to PCM buffers and reports, for every word, the
// sample-accurate position in that audio stream (AVSpeechSynthesisMarker,
// mark == .word, byteSampleOffset). Those positions are the ground truth; the
// audio is resampled to 48 kHz mono s16 with a zero-phase windowed-sinc
// filter, so marker seconds and WAV seconds share one timebase.
// Compiled into target/ by scripts/build_groundtruth.sh.
//
//   tts_groundtruth <out-dir> --name <stem> [--voice <identifier>] [--rate 0.5] [--script-id 1..4]
//   tts_groundtruth --list-voices
//
// Writes <stem>.wav, <stem>.words.json ([{"word","start","end"}], seconds;
// end = next word's start, the audio end for the last word), <stem>.script.txt
// (the exact text spoken) and <stem>.meta.json.
//
// Markers: write(_:toBufferCallback:toMarkerCallback:) is the source. On some
// macOS releases (observed on 26.5) that marker callback is never invoked;
// the same AVSpeechSynthesisMarker objects (same byteSampleOffset) then arrive
// through the delegate's speechSynthesizer(_:willSpeak:utterance:) (macOS 14+)
// during the same write() call, and are used instead (meta "marker_source").
// Timing never comes from willSpeakRangeOfSpeechString (wall clock).
//
// Exit codes: 0 ok, 1 error, 2 usage.
import AVFoundation
import Foundation

let outputRate = 48_000.0
let timeoutSeconds = 180.0

let scripts: [String] = [
    // 1
    "In 1969, a rocket burned for eight minutes and carried three people toward the Moon. "
        + "Back home, 23 percent of Americans still doubted the landing. "
        + "Neil Armstrong's first words were simple: \"one small step.\" "
        + "Today, that program would cost about $381 billion, "
        + "and its engineers' notebooks fill entire museum rooms.",
    // 2
    "Every year, the world spends three hundred and eighty one billion dollars on coffee. "
        + "That's more than most countries earn. "
        + "Prices rose 23 percent since 1969, yet growers keep less of every cup. "
        + "One trader called it \"a bitter bargain.\" "
        + "Nearly 4.5 million families depend on those beans. "
        + "Their harvest takes months, and your cup lasts eight minutes.",
    // 3
    "Sunlight takes eight minutes to reach us, so every sunrise you see is already old news. "
        + "The Sun's core burns 4.5 million tonnes of matter each second. "
        + "Since 1969, satellites have measured its storms, "
        + "and one scientist called it \"a patient fire.\" "
        + "A single bad flare could cost $381 billion when it knocks out our power grids.",
    // 4
    "It's midnight, and your brain is replaying one awkward mistake. "
        + "Researchers asked students to guess how many people noticed. They said half. "
        + "Only 23 percent did. Your mind's spotlight feels enormous, "
        + "but everyone else's attention is busy elsewhere. "
        + "Psychologists call it \"the spotlight effect.\" "
        + "Across 4.5 million online posts, the same pattern appears: "
        + "we overestimate how much others watch us.",
]

let usageText = """
    usage: tts_groundtruth <out-dir> --name <stem> [--voice <identifier>] [--rate 0.5] [--script-id 1..4]
           tts_groundtruth --list-voices

    """

func fail(_ msg: String, _ code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data(("tts_groundtruth: " + msg + "\n").utf8))
    exit(code)
}

func usage(_ msg: String) -> Never {
    FileHandle.standardError.write(Data(("tts_groundtruth: " + msg + "\n" + usageText).utf8))
    exit(2)
}

// MARK: - Voices

func genderName(_ g: AVSpeechSynthesisVoiceGender) -> String {
    switch g {
    case .male: return "male"
    case .female: return "female"
    default: return "unspecified"
    }
}

func qualityName(_ q: AVSpeechSynthesisVoiceQuality) -> String {
    switch q {
    case .premium: return "premium"
    case .enhanced: return "enhanced"
    default: return "default"
    }
}

func listVoices() -> Never {
    let voices = AVSpeechSynthesisVoice.speechVoices()
        .filter { $0.language == "en-US" || $0.language == "en-GB" }
        .sorted {
            ($0.language, -$0.quality.rawValue, $0.name, $0.identifier)
                < ($1.language, -$1.quality.rawValue, $1.name, $1.identifier)
        }
    print("identifier\tname\tlanguage\tgender\tquality")
    for v in voices {
        print("\(v.identifier)\t\(v.name)\t\(v.language)\t\(genderName(v.gender))\t\(qualityName(v.quality))")
    }
    exit(0)
}

// MARK: - Arguments

struct Options {
    var outDir = ""
    var name = ""
    var voice: String?
    var rate: Float = 0.5
    var scriptId = 1
}

func parseArgs() -> Options {
    var args = Array(CommandLine.arguments.dropFirst())
    if args.isEmpty { usage("missing arguments") }
    if args.contains("--help") || args.contains("-h") {
        print(usageText, terminator: "")
        exit(0)
    }
    if args.contains("--list-voices") {
        if args.count != 1 { usage("--list-voices takes no other arguments") }
        listVoices()
    }
    var o = Options()
    var positional: [String] = []
    while !args.isEmpty {
        let a = args.removeFirst()
        func value() -> String {
            if args.isEmpty { usage("\(a) needs a value") }
            return args.removeFirst()
        }
        switch a {
        case "--name": o.name = value()
        case "--voice": o.voice = value()
        case "--rate":
            let v = value()
            guard let r = Float(v), r >= AVSpeechUtteranceMinimumSpeechRate,
                r <= AVSpeechUtteranceMaximumSpeechRate
            else { usage("--rate must be a number in 0...1, got \(v)") }
            o.rate = r
        case "--script-id":
            let v = value()
            guard let n = Int(v), n >= 1, n <= scripts.count else {
                usage("--script-id must be 1...\(scripts.count), got \(v)")
            }
            o.scriptId = n
        default:
            if a.hasPrefix("-") { usage("unknown option \(a)") }
            positional.append(a)
        }
    }
    if positional.count != 1 { usage("expected exactly one <out-dir>") }
    o.outDir = positional[0]
    if o.name.isEmpty || o.name.contains("/") { usage("--name <stem> is required (no '/')") }
    return o
}

// MARK: - Capture

/// Collects every PCM buffer (as mono float) and every marker of one write().
final class Capture: NSObject, AVSpeechSynthesizerDelegate {
    private let lock = NSLock()
    private(set) var mono: [Float] = []
    private(set) var nativeRate = 0.0
    private(set) var bytesPerFrame: UInt32 = 0
    private(set) var formatDescription = ""
    private(set) var callbackMarkers: [AVSpeechSynthesisMarker] = []
    private(set) var delegateMarkers: [AVSpeechSynthesisMarker] = []
    private(set) var problem: String?
    private var lastBuffer = false
    private var finished = false

    var lastBufferSeen: Bool { lock.lock(); defer { lock.unlock() }; return lastBuffer }
    var finishSeen: Bool { lock.lock(); defer { lock.unlock() }; return finished }

    func append(_ buffer: AVAudioBuffer) {
        lock.lock(); defer { lock.unlock() }
        guard let pcm = buffer as? AVAudioPCMBuffer else {
            problem = problem ?? "synthesiser delivered a non-PCM buffer"
            return
        }
        let f = pcm.format
        let asbd = f.streamDescription.pointee
        if nativeRate == 0 {
            nativeRate = f.sampleRate
            bytesPerFrame = asbd.mBytesPerFrame
            formatDescription =
                "common=\(f.commonFormat.rawValue) channels=\(f.channelCount) "
                + "interleaved=\(f.isInterleaved) bytes_per_frame=\(asbd.mBytesPerFrame) rate=\(f.sampleRate)"
        } else if f.sampleRate != nativeRate || asbd.mBytesPerFrame != bytesPerFrame {
            problem = problem ?? "buffer format changed mid-stream"
        }
        let n = Int(pcm.frameLength)
        if n == 0 {
            lastBuffer = true
            return
        }
        let ch = Int(f.channelCount)
        let stride = pcm.stride
        let inter = f.isInterleaved
        func mix(_ sample: (Int, Int) -> Float) {
            for i in 0..<n {
                var acc: Float = 0
                for c in 0..<ch { acc += sample(c, i) }
                mono.append(acc / Float(max(ch, 1)))
            }
        }
        switch f.commonFormat {
        case .pcmFormatFloat32:
            guard let d = pcm.floatChannelData else { problem = problem ?? "no float data"; return }
            mix { c, i in inter ? d[0][i * stride + c] : d[c][i * stride] }
        case .pcmFormatInt16:
            guard let d = pcm.int16ChannelData else { problem = problem ?? "no int16 data"; return }
            mix { c, i in Float(inter ? d[0][i * stride + c] : d[c][i * stride]) / 32768 }
        case .pcmFormatInt32:
            guard let d = pcm.int32ChannelData else { problem = problem ?? "no int32 data"; return }
            mix { c, i in Float(inter ? d[0][i * stride + c] : d[c][i * stride]) / 2_147_483_648 }
        default:
            problem = problem ?? "unsupported PCM format \(f.commonFormat.rawValue)"
        }
    }

    func addCallbackMarkers(_ markers: [AVSpeechSynthesisMarker]) {
        lock.lock(); defer { lock.unlock() }
        callbackMarkers.append(contentsOf: markers)
    }

    @available(macOS 14.0, *)
    func speechSynthesizer(
        _ synthesizer: AVSpeechSynthesizer, willSpeak marker: AVSpeechSynthesisMarker,
        utterance: AVSpeechUtterance
    ) {
        lock.lock(); defer { lock.unlock() }
        delegateMarkers.append(marker)
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        lock.lock(); defer { lock.unlock() }
        finished = true
    }
}

func pump(until done: () -> Bool, deadline: Date) -> Bool {
    while !done() {
        if Date() > deadline { return false }
        _ = RunLoop.main.run(mode: .default, before: Date(timeIntervalSinceNow: 0.02))
    }
    return true
}

// MARK: - Resampling (zero-phase windowed sinc)

/// Output sample n sits at t = n / outRate, i.e. input position n * inRate / outRate;
/// the symmetric kernel adds no delay, so t = 0 is the same instant in both streams.
func resample(_ x: [Float], from inRate: Double, to outRate: Double) -> [Float] {
    if inRate == outRate { return x }
    let step = inRate / outRate
    let scale = min(1.0, outRate / inRate)
    let fc = 0.5 * 0.97 * scale  // cutoff, cycles per input sample
    let halfWidth = 16.0 / scale  // input samples on each side
    let outCount = Int((Double(x.count) * outRate / inRate).rounded())
    var y = [Float](repeating: 0, count: outCount)
    for n in 0..<outCount {
        let p = Double(n) * step
        let k0 = Int((p - halfWidth).rounded(.up))
        let k1 = Int((p + halfWidth).rounded(.down))
        var acc = 0.0
        var norm = 0.0
        if k0 <= k1 {
            for k in k0...k1 {
                let u = p - Double(k)
                let z = 2 * fc * u
                let sinc = abs(z) < 1e-12 ? 1.0 : sin(Double.pi * z) / (Double.pi * z)
                let w = 0.42 + 0.5 * cos(Double.pi * u / halfWidth) + 0.08 * cos(2 * Double.pi * u / halfWidth)
                let h = 2 * fc * sinc * w
                norm += h
                if k >= 0 && k < x.count { acc += Double(x[k]) * h }
            }
        }
        y[n] = norm > 0 ? Float(acc / norm) : 0
    }
    return y
}

// MARK: - Output helpers

func wavBytes(_ samples: [Int16], rate: UInt32) -> Data {
    var d = Data()
    func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
    func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
    let dataLen = UInt32(samples.count * 2)
    d.append(contentsOf: Array("RIFF".utf8)); u32(36 + dataLen)
    d.append(contentsOf: Array("WAVEfmt ".utf8)); u32(16)
    u16(1); u16(1); u32(rate); u32(rate * 2); u16(2); u16(16)
    d.append(contentsOf: Array("data".utf8)); u32(dataLen)
    for s in samples { withUnsafeBytes(of: s.littleEndian) { d.append(contentsOf: $0) } }
    return d
}

func micro(_ x: Double) -> Double { (x * 1e6).rounded() / 1e6 }

/// The engine's token rule (motion_voice::fixture::strip_punct): trim everything
/// that is not alphanumeric or one of % ₹ $ € £ +.
func stripPunct(_ s: String) -> String {
    let keep: Set<Character> = ["%", "₹", "$", "€", "£", "+"]
    let isKept: (Character) -> Bool = { $0.isLetter || $0.isNumber || keep.contains($0) }
    guard let a = s.firstIndex(where: isKept), let b = s.lastIndex(where: isKept) else { return "" }
    return String(s[a...b])
}

struct GroundTruthWord: Codable {
    let word: String
    let start: Double
    let end: Double
}

struct RawMarker: Codable {
    let byte_offset: Int
    let frame: Int
    let seconds: Double
    let range: [Int]
    let text: String
}

struct Meta: Codable {
    let voice_identifier: String
    let voice_name: String
    let language: String
    let gender: String
    let quality: String
    let rate: Double
    let script_id: Int
    let macos_version: String
    let native_sample_rate: Double
    let native_format: String
    let native_frames: Int
    let output_sample_rate: Double
    let output_samples: Int
    let duration: Double
    let resampler: String
    let peak: Double
    let clipped_samples: Int
    let marker_source: String
    let markers_total: Int
    let word_markers: Int
    let markers_frame_aligned: Bool
    let script_tokens: Int
    let gt_words: Int
    let tokens_without_marker: [String]
    let extra_markers_in_token: Int
    let unmapped_markers: Int
    let non_monotone_dropped: Int
    let raw_word_markers: [RawMarker]
}

func writeJSON<T: Encodable>(_ value: T, to url: URL) throws {
    let enc = JSONEncoder()
    enc.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    var data = try enc.encode(value)
    data.append(0x0A)
    try data.write(to: url, options: .atomic)
}

// MARK: - Main

let opts = parseArgs()
let script = scripts[opts.scriptId - 1]

let voice: AVSpeechSynthesisVoice
if let id = opts.voice {
    guard let v = AVSpeechSynthesisVoice(identifier: id) else {
        fail("unknown voice \(id) (see --list-voices)")
    }
    voice = v
} else {
    guard let v = AVSpeechSynthesisVoice(language: "en-US") else { fail("no en-US voice installed") }
    voice = v
}

let capture = Capture()
let synth = AVSpeechSynthesizer()
synth.delegate = capture
let utterance = AVSpeechUtterance(string: script)
utterance.voice = voice
utterance.rate = opts.rate
utterance.pitchMultiplier = 1.0
utterance.volume = 1.0
utterance.preUtteranceDelay = 0
utterance.postUtteranceDelay = 0

synth.write(
    utterance,
    toBufferCallback: { capture.append($0) },
    toMarkerCallback: { capture.addCallbackMarkers($0) })

let deadline = Date(timeIntervalSinceNow: timeoutSeconds)
if !pump(until: { capture.lastBufferSeen }, deadline: deadline) {
    fail("synthesis timed out after \(Int(timeoutSeconds)) s")
}
// Markers delivered through the delegate may trail the last buffer; wait for
// didFinish (bounded), then drain the run loop briefly.
_ = pump(until: { capture.finishSeen }, deadline: Date(timeIntervalSinceNow: 3))
_ = pump(until: { false }, deadline: Date(timeIntervalSinceNow: 0.25))

if let p = capture.problem { fail(p) }
let native = capture.mono
let nativeRate = capture.nativeRate
let bytesPerFrame = Int(capture.bytesPerFrame)
if native.isEmpty || nativeRate <= 0 || bytesPerFrame <= 0 {
    fail("voice \(voice.identifier) produced no audio")
}

let callbackWords = capture.callbackMarkers.filter { $0.mark == .word }
let delegateWords = capture.delegateMarkers.filter { $0.mark == .word }
let markerSource: String
let allMarkers: [AVSpeechSynthesisMarker]
if !callbackWords.isEmpty {
    markerSource = "write_marker_callback"
    allMarkers = capture.callbackMarkers
} else {
    markerSource = "delegate_willSpeak_marker"
    allMarkers = capture.delegateMarkers
}
let wordMarkers = allMarkers.filter { $0.mark == .word }
if wordMarkers.isEmpty {
    fail("voice \(voice.identifier) produced no word markers (callback: \(capture.callbackMarkers.count) markers, delegate: \(capture.delegateMarkers.count))")
}

// Marker byte offset -> native frame -> seconds.
let nativeFrames = native.count
let sorted = wordMarkers.enumerated().sorted {
    ($0.element.byteSampleOffset, $0.offset) < ($1.element.byteSampleOffset, $1.offset)
}.map { $0.element }
var aligned = true
for m in sorted {
    if m.byteSampleOffset % bytesPerFrame != 0 { aligned = false }
    if m.byteSampleOffset / bytesPerFrame > nativeFrames {
        fail("marker at byte \(m.byteSampleOffset) lies beyond the audio (\(nativeFrames) frames x \(bytesPerFrame) bytes)")
    }
}

// Script tokens (whitespace-delimited, UTF-16 offsets as in NSRange).
let units = Array(script.utf16)
func isSpace(_ u: UInt16) -> Bool { u == 0x20 || u == 0x09 || u == 0x0A || u == 0x0D || u == 0xA0 }
var tokenOf = [Int](repeating: -1, count: units.count)
var tokenRanges: [(Int, Int)] = []
var ui = 0
while ui < units.count {
    if isSpace(units[ui]) { ui += 1; continue }
    let s = ui
    while ui < units.count && !isSpace(units[ui]) { tokenOf[ui] = tokenRanges.count; ui += 1 }
    tokenRanges.append((s, ui))
}
let ns = script as NSString
let tokenText = tokenRanges.map { stripPunct(ns.substring(with: NSRange(location: $0.0, length: $0.1 - $0.0))) }

// One ground-truth word per script token: the earliest word marker whose text
// range starts in that token. Several markers in one token (a number read as
// several words, "$381 billion" -> "... billion dollars") keep the first; a
// token no marker starts in (the "billion" of "$381 billion") stays untimed.
var tokenStart = [Double?](repeating: nil, count: tokenRanges.count)
var extraInToken = 0
var unmapped = 0
for m in sorted {
    var loc = m.textRange.location
    if loc == NSNotFound || loc < 0 { unmapped += 1; continue }
    while loc < units.count && tokenOf[loc] < 0 { loc += 1 }
    if loc >= units.count { unmapped += 1; continue }
    let t = tokenOf[loc]
    if tokenStart[t] != nil { extraInToken += 1; continue }
    tokenStart[t] = Double(m.byteSampleOffset / bytesPerFrame) / nativeRate
}

let duration = Double(nativeFrames) / nativeRate
var starts: [(String, Double)] = []
var missing: [String] = []
var dropped = 0
for (t, text) in tokenText.enumerated() where !text.isEmpty {
    guard let s = tokenStart[t] else { missing.append(text); continue }
    if let last = starts.last, s < last.1 { dropped += 1; continue }
    starts.append((text, s))
}
var words: [GroundTruthWord] = []
for (i, w) in starts.enumerated() {
    let end = i + 1 < starts.count ? starts[i + 1].1 : duration
    words.append(GroundTruthWord(word: w.0, start: micro(w.1), end: micro(end)))
}

// 48 kHz mono s16.
let out = resample(native, from: nativeRate, to: outputRate)
var peak: Float = 0
var clipped = 0
var pcm = [Int16](repeating: 0, count: out.count)
for (i, v) in out.enumerated() {
    peak = max(peak, abs(v))
    let q = (Double(v) * 32767).rounded()
    if q > 32767 || q < -32768 { clipped += 1 }
    pcm[i] = Int16(max(-32768, min(32767, q)))
}

let dir = URL(fileURLWithPath: opts.outDir, isDirectory: true)
let tokensCount = tokenText.filter { !$0.isEmpty }.count
let meta = Meta(
    voice_identifier: voice.identifier,
    voice_name: voice.name,
    language: voice.language,
    gender: genderName(voice.gender),
    quality: qualityName(voice.quality),
    rate: micro(Double(opts.rate)),
    script_id: opts.scriptId,
    macos_version: ProcessInfo.processInfo.operatingSystemVersionString,
    native_sample_rate: nativeRate,
    native_format: capture.formatDescription,
    native_frames: nativeFrames,
    output_sample_rate: outputRate,
    output_samples: pcm.count,
    duration: micro(Double(pcm.count) / outputRate),
    resampler: "windowed sinc (Blackman, 16 zero crossings, cutoff 0.97 x min Nyquist), zero phase",
    peak: Double(peak),
    clipped_samples: clipped,
    marker_source: markerSource,
    markers_total: allMarkers.count,
    word_markers: wordMarkers.count,
    markers_frame_aligned: aligned,
    script_tokens: tokensCount,
    gt_words: words.count,
    tokens_without_marker: missing,
    extra_markers_in_token: extraInToken,
    unmapped_markers: unmapped,
    non_monotone_dropped: dropped,
    raw_word_markers: sorted.map { m in
        let r = m.textRange
        let ok = r.location != NSNotFound && r.location >= 0 && r.location + r.length <= ns.length
        let frame = m.byteSampleOffset / bytesPerFrame
        return RawMarker(
            byte_offset: m.byteSampleOffset, frame: frame, seconds: micro(Double(frame) / nativeRate),
            range: [ok ? r.location : -1, r.length], text: ok ? ns.substring(with: r) : "")
    })

do {
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    try wavBytes(pcm, rate: UInt32(outputRate)).write(
        to: dir.appendingPathComponent("\(opts.name).wav"), options: .atomic)
    try writeJSON(words, to: dir.appendingPathComponent("\(opts.name).words.json"))
    try Data(script.utf8).write(
        to: dir.appendingPathComponent("\(opts.name).script.txt"), options: .atomic)
    try writeJSON(meta, to: dir.appendingPathComponent("\(opts.name).meta.json"))
} catch {
    fail("writing outputs: \(error)")
}
print(
    "\(opts.name): voice \(voice.identifier), rate \(opts.rate), script \(opts.scriptId), "
        + String(format: "%.2f s", duration)
        + ", \(wordMarkers.count) word markers (\(markerSource)), \(words.count)/\(tokensCount) tokens timed")
exit(0)

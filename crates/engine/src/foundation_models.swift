import Foundation
import FoundationModels

public typealias ModelText = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>) -> Bool
public typealias ModelDone = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Void

/// 0 ready, 1 this device cannot run it, 2 Apple Intelligence is off, 3 the model is not
/// downloaded yet, 4 the system is older than the framework.
@_cdecl("tarn_model_availability")
public func tarnModelAvailability() -> Int32 {
    guard #available(macOS 26.0, iOS 26.0, *) else { return 4 }
    switch SystemLanguageModel.default.availability {
    case .available: return 0
    case .unavailable(.appleIntelligenceNotEnabled): return 2
    case .unavailable(.modelNotReady): return 3
    case .unavailable: return 1
    }
}

/// Answers `prompt` with the system's on-device model only. `text` gets each new piece and returns
/// false to stop; `done` runs exactly once, after the last `text`, with nil or an error code.
@_cdecl("tarn_model_respond")
public func tarnModelRespond(_ instructions: UnsafePointer<CChar>, _ prompt: UnsafePointer<CChar>, _ context: UnsafeMutableRawPointer?, _ text: ModelText, _ done: ModelDone) {
    let (instructions, prompt) = (String(cString: instructions), String(cString: prompt))
    let context = UInt(bitPattern: context)
    Task.detached {
        let error = await respond(instructions, prompt) { piece in piece.withCString { text(UnsafeMutableRawPointer(bitPattern: context), $0) } }
        if let error {
            error.withCString { done(UnsafeMutableRawPointer(bitPattern: context), $0) }
        } else {
            done(UnsafeMutableRawPointer(bitPattern: context), nil)
        }
    }
}

private func respond(_ instructions: String, _ prompt: String, _ text: (String) -> Bool) async -> String? {
    guard #available(macOS 26.0, iOS 26.0, *) else { return "unavailable" }
    let session = LanguageModelSession(model: SystemLanguageModel.default, instructions: instructions)
    var sent = ""
    do {
        for try await snapshot in session.streamResponse(to: prompt) {
            let piece = snapshot.content.hasPrefix(sent) ? String(snapshot.content.dropFirst(sent.count)) : ""
            sent = snapshot.content
            if !piece.isEmpty && !text(piece) {
                return nil
            }
        }
        return nil
    } catch let error as LanguageModelSession.GenerationError {
        switch error {
        case .guardrailViolation, .refusal: return "refused"
        case .exceededContextWindowSize: return "too long"
        case .unsupportedLanguageOrLocale: return "language"
        default: return "failed"
        }
    } catch {
        return "failed"
    }
}

import Foundation
import Translation

public typealias TranslateDone = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>) -> Void

/// Translates a JSON array of strings from `source` to `target` with language packs already
/// installed on this Mac; `done` gets the JSON array of translations in the same order, or an
/// error text that is not a JSON array. Runs `done` exactly once, on any thread.
@_cdecl("tarn_translate")
public func tarnTranslate(_ texts: UnsafePointer<CChar>, _ source: UnsafePointer<CChar>, _ target: UnsafePointer<CChar>, _ context: UnsafeMutableRawPointer?, _ done: TranslateDone) {
    let (json, source, target) = (String(cString: texts), String(cString: source), String(cString: target))
    let context = UInt(bitPattern: context)
    Task.detached {
        let result = await translate(json, source, target)
        result.withCString { done(UnsafeMutableRawPointer(bitPattern: context), $0) }
    }
}

private func translate(_ json: String, _ source: String, _ target: String) async -> String {
    guard #available(macOS 26.0, *) else { return "failed" }
    guard let texts = try? JSONDecoder().decode([String].self, from: Data(json.utf8)) else { return "bad input" }
    let session = TranslationSession(installedSource: Locale.Language(identifier: source), target: Locale.Language(identifier: target))
    do {
        let requests = texts.enumerated().map { TranslationSession.Request(sourceText: $1, clientIdentifier: String($0)) }
        var out = texts
        for response in try await session.translations(from: requests) {
            if let i = response.clientIdentifier.flatMap(Int.init), out.indices.contains(i) {
                out[i] = response.targetText
            }
        }
        return String(decoding: try JSONEncoder().encode(out), as: UTF8.self)
    } catch let error where TranslationError.notInstalled ~= error {
        return "not installed"
    } catch {
        return "failed"
    }
}

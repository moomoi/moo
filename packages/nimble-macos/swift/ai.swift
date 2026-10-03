// Apple's on-device model (FoundationModels, macOS 26+) behind a C ABI for `src/ai.rs`.
// FoundationModels is Swift-only, so this is the one Swift file in Nimble. It is compiled into a
// static library by build.rs; every use is behind `#available`, so the framework is weak-linked and
// the app still launches on older macOS, where `nimble_ai_availability` reports why AI is off.

import Foundation
import FoundationModels

/// `(request, kind, text)`. kind: 0 partial (the whole reply so far), 1 done (final text),
/// 2 error (message), 3 cancelled. Called on a Swift concurrency thread.
public typealias NimbleAICallback = @convention(c) (UInt64, Int32, UnsafePointer<CChar>?) -> Void

private let lock = NSLock()
nonisolated(unsafe) private var sessions: [UInt64: AnyObject] = [:]
nonisolated(unsafe) private var tasks: [UInt64: Task<Void, Never>] = [:]
nonisolated(unsafe) private var nextSession: UInt64 = 1

private func locked<T>(_ f: () -> T) -> T {
    lock.lock()
    defer { lock.unlock() }
    return f()
}

private func emit(_ cb: NimbleAICallback, _ request: UInt64, _ kind: Int32, _ text: String?) {
    guard let text else { return cb(request, kind, nil) }
    text.withCString { cb(request, kind, $0) }
}

/// "available", or why not (a malloc'd string; free with `nimble_ai_free`).
@_cdecl("nimble_ai_availability")
public func nimble_ai_availability() -> UnsafeMutablePointer<CChar>? {
    guard #available(macOS 26.0, *) else { return strdup("requires macOS 26") }
    switch SystemLanguageModel.default.availability {
    case .available:
        return strdup("available")
    case .unavailable(let reason):
        return strdup(String(describing: reason))
    }
}

@_cdecl("nimble_ai_free")
public func nimble_ai_free(_ p: UnsafeMutablePointer<CChar>?) {
    free(p)
}

/// A conversation: later asks see earlier turns. Returns 0 when the model is unavailable.
@_cdecl("nimble_ai_session_new")
public func nimble_ai_session_new(_ instructions: UnsafePointer<CChar>?) -> UInt64 {
    guard #available(macOS 26.0, *), case .available = SystemLanguageModel.default.availability else { return 0 }
    let text = instructions.map { String(cString: $0) } ?? ""
    let session = text.isEmpty ? LanguageModelSession() : LanguageModelSession(instructions: text)
    return locked {
        let id = nextSession
        nextSession += 1
        sessions[id] = session
        return id
    }
}

@_cdecl("nimble_ai_session_free")
public func nimble_ai_session_free(_ id: UInt64) {
    _ = locked { sessions.removeValue(forKey: id) }
}

/// Load the model ahead of the first ask (the first reply after a cold start takes about 2 s).
@_cdecl("nimble_ai_prewarm")
public func nimble_ai_prewarm(_ id: UInt64) {
    guard #available(macOS 26.0, *) else { return }
    (locked { sessions[id] } as? LanguageModelSession)?.prewarm()
}

/// Stream a reply to `prompt` in session `id`, reporting through `cb` under `request`.
/// Returns false if the session does not exist or is already answering.
@_cdecl("nimble_ai_ask")
public func nimble_ai_ask(_ id: UInt64, _ request: UInt64, _ prompt: UnsafePointer<CChar>, _ cb: NimbleAICallback) -> Bool {
    guard #available(macOS 26.0, *) else { return false }
    guard let session = locked({ sessions[id] }) as? LanguageModelSession, !session.isResponding else { return false }
    let job = Job(session: session, cb: cb)
    let text = String(cString: prompt)
    // Hold the lock until the task is recorded, so its own removal cannot run first.
    lock.lock()
    defer { lock.unlock() }
    tasks[request] = Task.detached {
        let cb = job.cb
        var last = ""
        do {
            for try await snapshot in job.session.streamResponse(to: text) {
                if Task.isCancelled { break }
                last = snapshot.content
                emit(cb, request, 0, last)
            }
            emit(cb, request, Task.isCancelled ? 3 : 1, last)
        } catch is CancellationError {
            emit(cb, request, 3, last)
        } catch {
            emit(cb, request, 2, String(describing: error))
        }
        _ = locked { tasks.removeValue(forKey: request) }
    }
    return true
}

/// The Rust callback is thread-safe (it only queues), and the session is used by one task at a time.
@available(macOS 26.0, *)
private final class Job: @unchecked Sendable {
    let session: LanguageModelSession
    let cb: NimbleAICallback
    init(session: LanguageModelSession, cb: NimbleAICallback) {
        self.session = session
        self.cb = cb
    }
}

@_cdecl("nimble_ai_cancel")
public func nimble_ai_cancel(_ request: UInt64) {
    locked { tasks[request] }?.cancel()
}

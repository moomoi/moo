// HTTP for `src/http.rs` over URLSession, behind a C ABI. Responses stream line by line, which is
// what server-sent events (AI replies) need; whole bodies are just their lines joined.

import Foundation

/// `(request, kind, status, text)`. kind: 0 response started (status), 1 a line of the body,
/// 2 finished, 3 failed (message), 4 cancelled. Called on a Swift concurrency thread.
public typealias MooHTTPCallback = @convention(c) (UInt64, Int32, Int32, UnsafePointer<CChar>?) -> Void

private let httpLock = NSLock()
nonisolated(unsafe) private var httpTasks: [UInt64: Task<Void, Never>] = [:]

private let httpSession: URLSession = {
    let c = URLSessionConfiguration.ephemeral
    c.requestCachePolicy = .reloadIgnoringLocalCacheData
    c.httpAdditionalHeaders = ["User-Agent": "Moo"]
    return URLSession(configuration: c)
}()

private func send(_ cb: MooHTTPCallback, _ id: UInt64, _ kind: Int32, _ status: Int32, _ text: String?) {
    guard let text else { return cb(id, kind, status, nil) }
    text.withCString { cb(id, kind, status, $0) }
}

/// Start a request. `headers` is a JSON object of strings; `timeout` is the longest wait for data
/// (an idle timeout, so long streams are fine). Returns false for an invalid URL.
@_cdecl("moo_http_start")
public func moo_http_start(
    _ id: UInt64, _ method: UnsafePointer<CChar>, _ url: UnsafePointer<CChar>, _ headers: UnsafePointer<CChar>,
    _ body: UnsafePointer<UInt8>?, _ bodyLen: Int, _ timeout: Double, _ cb: MooHTTPCallback
) -> Bool {
    guard let u = URL(string: String(cString: url)) else { return false }
    var req = URLRequest(url: u)
    req.httpMethod = String(cString: method)
    req.timeoutInterval = timeout > 0 ? timeout : 60
    if let data = String(cString: headers).data(using: .utf8),
       let h = (try? JSONSerialization.jsonObject(with: data)) as? [String: String] {
        for (k, v) in h { req.setValue(v, forHTTPHeaderField: k) }
    }
    if let body, bodyLen > 0 { req.httpBody = Data(bytes: body, count: bodyLen) }
    let job = HTTPJob(request: req, cb: cb)
    httpLock.lock()
    defer { httpLock.unlock() }
    httpTasks[id] = Task.detached {
        let (request, cb) = (job.request, job.cb)
        do {
            let (bytes, response) = try await httpSession.bytes(for: request)
            send(cb, id, 0, Int32((response as? HTTPURLResponse)?.statusCode ?? 0), nil)
            for try await line in bytes.lines {
                if Task.isCancelled { break }
                send(cb, id, 1, 0, line)
            }
            send(cb, id, Task.isCancelled ? 4 : 2, 0, nil)
        } catch is CancellationError {
            send(cb, id, 4, 0, nil)
        } catch let e as URLError where e.code == .cancelled {
            send(cb, id, 4, 0, nil)
        } catch {
            send(cb, id, 3, 0, error.localizedDescription)
        }
        forget(id)
    }
    return true
}

/// The Rust callback is thread-safe (it only takes a lock and runs a handler).
private final class HTTPJob: @unchecked Sendable {
    let request: URLRequest
    let cb: MooHTTPCallback
    init(request: URLRequest, cb: MooHTTPCallback) {
        self.request = request
        self.cb = cb
    }
}

private func forget(_ id: UInt64) {
    httpLock.lock()
    httpTasks.removeValue(forKey: id)
    httpLock.unlock()
}

@_cdecl("moo_http_cancel")
public func moo_http_cancel(_ id: UInt64) {
    httpLock.lock()
    let t = httpTasks[id]
    httpLock.unlock()
    t?.cancel()
}

// xpress-ai — a tiny helper that gives xpress access to Apple Intelligence
// (the on-device Foundation Models). The app runs it as a separate process
// so the main binary keeps working on macOS versions without the framework.
//
//   xpress-ai status                              → available | not-enabled |
//                                                   device-not-eligible |
//                                                   model-not-ready | unavailable
//   xpress-ai respond --instructions TEXT < input → the model's answer on stdout
//
// Exit codes for `respond`: 0 ok, 1 error, 2 unavailable, 3 declined
// (guardrails), 4 too long, 64 usage.

import Foundation
import FoundationModels

func fail(_ message: String, code: Int32) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

func status(_ model: SystemLanguageModel) -> String {
    switch model.availability {
    case .available:
        return "available"
    case .unavailable(.appleIntelligenceNotEnabled):
        return "not-enabled"
    case .unavailable(.deviceNotEligible):
        return "device-not-eligible"
    case .unavailable(.modelNotReady):
        return "model-not-ready"
    case .unavailable:
        return "unavailable"
    }
}

@main
struct XpressAI {
    static func main() async {
        let args = Array(CommandLine.arguments.dropFirst())
        let model = SystemLanguageModel.default
        switch args.first {
        case "status":
            print(status(model))
        case "respond":
            var instructions = ""
            if let i = args.firstIndex(of: "--instructions"), i + 1 < args.count {
                instructions = args[i + 1]
            }
            let input = String(
                decoding: FileHandle.standardInput.readDataToEndOfFile(), as: UTF8.self)
            guard case .available = model.availability else {
                fail("Apple Intelligence isn't available (\(status(model))).", code: 2)
            }
            let session = LanguageModelSession(instructions: instructions)
            do {
                let response = try await session.respond(to: input)
                print(response.content, terminator: "")
            } catch let error as LanguageModelSession.GenerationError {
                switch error {
                case .exceededContextWindowSize:
                    fail("The text is too long for Apple Intelligence.", code: 4)
                case .guardrailViolation, .refusal:
                    fail("Apple Intelligence declined to work on this text.", code: 3)
                default:
                    fail(error.localizedDescription, code: 1)
                }
            } catch {
                fail(error.localizedDescription, code: 1)
            }
        default:
            fail("usage: xpress-ai status | xpress-ai respond --instructions TEXT < input", code: 64)
        }
    }
}

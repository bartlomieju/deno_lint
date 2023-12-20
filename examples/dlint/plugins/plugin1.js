export default function plugin({
    context
}) {
    return {
        name: "example_plugin1",
        onEnd() {
            context.addDiagnostics({
                filename: context.filename,
                code: "example_plugin",
                message: "Example Plugin diagnostics",
                location: {
                    start: {
                        line: 1,
                        column: 1,
                    },
                    end: {
                        line: 1,
                        column: 1,
                    },
                }
            })
        },
        visitor: {
            Program(node) {
                console.log("Program", node);
            }
        },
    }
}
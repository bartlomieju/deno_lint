// import { Visitor } from "ext:dlint/visitor.js"
Deno.core.print("Hello from plugin_host.js\n");

const getCtx = Deno.core.ops.op_get_ctx;
const addDiagnostic = Deno.core.ops.op_add_diagnostic;

const loadedPlugins = [];
const context = {};

async function hostInit({ plugins }) {
    
    for (const pluginPath of plugins) {
        const pluginMod = await import(pluginPath);
        const pluginInstance = pluginMod.default({ context });
        loadedPlugins.push(pluginInstance);
    }

    Deno.core.print(`Loaded plugins: ${loadedPlugins.length}\n`)
}

function hostRequest() {
    const { filename, ast } = getCtx();
    Deno.core.print(`Got AST for ${filename}: ${JSON.stringify(ast, undefined, 4)}\n`);
    for (const plugin of loadedPlugins) {
        Deno.core.print(`Running plugin: ${plugin.name} for ${filename}\n`)
        // const visitor = new Visitor();
        // visitor.visitProgram(ast);
        if (ast.span) {
            addDiagnostic(plugin.name, "Example Plugin diagnostics", null, 100, 200);
        }
    }
}

globalThis.hostInit = hostInit;
globalThis.hostRequest = hostRequest;
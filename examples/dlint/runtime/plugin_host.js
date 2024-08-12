import { Visitor } from "ext:dlint/visitor.js"
console.log("Hello from plugin_host.js");

const getCtx = Deno.core.ops.op_get_ctx;
const getCtx2 = Deno.core.ops.op_get_ctx2;
const addDiagnostic = Deno.core.ops.op_add_diagnostic;

const loadedPlugins = [];
const context = {};

async function hostInit({ plugins }) {
    
    for (const pluginPath of plugins) {
        const pluginMod = await import(pluginPath);
        const pluginInstance = pluginMod.default({ context });
        loadedPlugins.push(pluginInstance);
    }

    console.log(`Loaded plugins: ${loadedPlugins.length}`)
}

function hostRequest() {
    // const [filename, ast] = getCtx();
    // const ast = getCtx();
    const ast = getCtx2();
    const filename = "..."
    // Deno.core.print(`Got AST for ${filename}: ${JSON.stringify(ast, undefined, 4)}\n`);
    console.log(`Got AST for ${filename}`);
    for (const plugin of loadedPlugins) {
        console.log(`Running plugin: ${plugin.name} for ${filename}`);
        // const visitor = new Visitor();
        // visitor.visitProgram(ast);
        if (ast.span) {
            addDiagnostic(plugin.name, "Example Plugin diagnostics", null, 100, 200);
        }
    }
}

globalThis.hostInit = hostInit;
globalThis.hostRequest = hostRequest;
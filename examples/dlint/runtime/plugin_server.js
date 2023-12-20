Deno.core.print("Hello from plugin_server.js\n");

const getCtx = Deno.core.ops.op_get_ctx;
const addDiagnostic = Deno.core.ops.op_add_diagnostic;

const loadedPlugins = [];
const context = {};

async function serverInit({ plugins }) {
    
    for (const pluginPath of plugins) {
        const pluginMod = await import(pluginPath);
        const pluginInstance = pluginMod.default({ context });
        loadedPlugins.push(pluginInstance);
    }

    Deno.core.print(`Loaded plugins: ${loadedPlugins.length}\n`)
}

function serverRequest() {
    const { filename, ast } = getCtx();
    Deno.core.print(`Got AST for ${filename}: ${JSON.stringify(ast)}\n`);
    for (const plugin of loadedPlugins) {
        Deno.core.print(`Running plugin: ${plugin.name} for ${filename}\n`)
        addDiagnostic(plugin.name, "Example Plugin diagnostics", 100, 200);
    }
}

globalThis.serverInit = serverInit;
globalThis.serverRequest = serverRequest;
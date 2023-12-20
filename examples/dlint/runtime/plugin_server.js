Deno.core.print("Hello from plugin_server.js\n");

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

function serverRequest({ filename }) {
    for (const plugin of loadedPlugins) {
        Deno.core.print(`Running plugin: ${plugin.name} for ${filename}\n`)
    }
}

globalThis.serverInit = serverInit;
globalThis.serverRequest = serverRequest;
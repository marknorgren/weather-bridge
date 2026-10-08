const $ = (id) => document.getElementById(id);
const text = (id, value) => {
    $(id).textContent = value;
};
let tools = [],
    activeTool = null,
    nextId = 0,
    connected = false,
    discovering = null;
const protocol = '2025-11-25';
function message(method, params) {
    return { jsonrpc: '2.0', id: ++nextId, method, ...(params ? { params } : {}) };
}
async function rpc(request) {
    const response = await fetch('/mcp', {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            Accept: 'application/json, text/event-stream',
            'MCP-Protocol-Version': protocol,
        },
        body: JSON.stringify(request),
        signal: AbortSignal.timeout(55000),
    });
    if (!response.ok) throw Error(`MCP returned HTTP ${response.status}. Reload or retry shortly.`);
    const body = await response.json();
    if (body.error) throw Error(body.error.message || 'MCP request failed.');
    return body.result;
}
async function discover() {
    await rpc(
        message('initialize', {
            protocolVersion: protocol,
            capabilities: {},
            clientInfo: { name: 'weather-bridge-explorer', version: '0.1.0' },
        }),
    );
    const initialized = await fetch('/mcp', {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            Accept: 'application/json, text/event-stream',
            'MCP-Protocol-Version': protocol,
        },
        body: JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }),
        signal: AbortSignal.timeout(10000),
    });
    if (!initialized.ok) throw Error('MCP initialization could not finish. Retry the MCP tab.');
    const result = await rpc(message('tools/list'));
    tools = result.tools;
    if (!Array.isArray(tools) || !tools.length)
        throw Error('No MCP tools were returned. Reload to reconnect.');
    connected = true;
    const list = $('tool-list');
    list.replaceChildren();
    for (const tool of tools) {
        const button = document.createElement('button');
        button.className = 'tool-button';
        button.type = 'button';
        button.setAttribute('aria-pressed', 'false');
        button.dataset.tool = tool.name;
        const name = document.createElement('strong');
        name.textContent = tool.name;
        const hint = document.createElement('span');
        hint.textContent = tool.annotations?.readOnlyHint ? 'Read-only tool' : 'MCP tool';
        button.append(name, hint);
        button.addEventListener('click', () => choose(tool));
        list.append(button);
    }
    $('connection').classList.remove('error');
    text('connection', `${tools.length} tools discovered · Connected to ${location.origin}/mcp`);
    choose(tools.find((t) => t.name === 'search_cities') || tools[0]);
}
async function tab(name, focus = false) {
    for (const id of ['rest', 'mcp']) {
        const selected = name === id;
        $(id + '-tab').setAttribute('aria-selected', String(selected));
        $(id + '-tab').tabIndex = selected ? 0 : -1;
        $(id + '-panel').hidden = !selected;
    }
    history.replaceState(null, '', name === 'mcp' ? '#mcp' : '#rest');
    if (focus) $(name + '-tab').focus();
    if (name === 'mcp' && !connected && !discovering) {
        discovering = discover()
            .catch((error) => {
                text('connection', error.message);
                $('connection').classList.add('error');
            })
            .finally(() => {
                discovering = null;
            });
        await discovering;
    }
}
$('rest-tab').addEventListener('click', () => {
    void tab('rest');
});
$('mcp-tab').addEventListener('click', () => {
    void tab('mcp');
});
for (const id of ['rest-tab', 'mcp-tab'])
    $(id).addEventListener('keydown', (e) => {
        if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) {
            e.preventDefault();
            void tab(
                e.key === 'Home'
                    ? 'rest'
                    : e.key === 'End'
                      ? 'mcp'
                      : id === 'rest-tab'
                        ? 'mcp'
                        : 'rest',
                true,
            );
        }
    });
function schemaType(schema, root) {
    if (schema.$ref?.startsWith('#/')) {
        const resolved = schema.$ref
            .slice(2)
            .split('/')
            .reduce((s, key) => s?.[key.replaceAll('~1', '/').replaceAll('~0', '~')], root);
        if (resolved) return schemaType(resolved, root);
    }
    if (schema.anyOf)
        return schemaType(schema.anyOf.find((s) => s.type !== 'null') || { type: 'string' }, root);
    return schema;
}
function choose(tool) {
    activeTool = tool;
    for (const b of $('tool-list').children)
        b.setAttribute('aria-pressed', String(b.dataset.tool === tool.name));
    $('tool-main').hidden = false;
    text('tool-name', tool.name);
    text('tool-description', tool.description || '');
    $('location-hint').hidden = !tool.inputSchema.properties?.city;
    $('fields').replaceChildren();
    $('use-json').checked = false;
    const required = tool.inputSchema.required || [];
    for (const [name, original] of Object.entries(tool.inputSchema.properties || {})) {
        const schema = schemaType(original, tool.inputSchema);
        const type = Array.isArray(schema.type)
            ? schema.type.find((t) => t !== 'null')
            : schema.type;
        const container = document.createElement('div');
        container.className = 'field';
        const label = document.createElement('label');
        label.htmlFor = `argument-${name}`;
        label.textContent = name + (required.includes(name) ? ' *' : '');
        const input = document.createElement(schema.enum ? 'select' : 'input');
        input.id = label.htmlFor;
        input.name = name;
        input.required = required.includes(name);
        input.dataset.type = type || 'string';
        if (schema.enum)
            for (const value of schema.enum) {
                const option = document.createElement('option');
                option.value = value;
                option.textContent = value;
                input.append(option);
            }
        else {
            input.type = ['number', 'integer'].includes(type) ? 'number' : 'text';
            if (input.type === 'number') input.step = type === 'integer' ? '1' : 'any';
        }
        if (schema.default !== null && schema.default !== undefined) input.value = schema.default;
        if (name === 'query') input.value = 'Springfield';
        if (name === 'city') input.value = 'Seattle, WA';
        const description = document.createElement('small');
        description.textContent =
            original.description ||
            schema.description ||
            (name === 'units' ? 'us = °F/mph · metric = °C/km/h' : 'Optional');
        container.append(label, input, description);
        $('fields').append(container);
    }
    $('arguments').value = JSON.stringify(fieldArguments(), null, 2);
    text('schema', JSON.stringify(tool, null, 2));
    $('result').hidden = true;
    $('result-title').hidden = true;
    $('request-details').hidden = true;
    text('call-status', '');
}
function fieldArguments() {
    const args = {};
    for (const input of $('fields').querySelectorAll('input,select')) {
        if (input.value.trim() === '') continue;
        const value = ['integer', 'number'].includes(input.dataset.type)
            ? Number(input.value)
            : input.value;
        if (typeof value === 'number' && !Number.isFinite(value))
            throw Error(`${input.name} must be a finite number.`);
        args[input.name] = value;
    }
    return args;
}
$('use-json').addEventListener('change', () => {
    for (const input of $('fields').querySelectorAll('input,select'))
        input.disabled = $('use-json').checked;
});
$('fields').addEventListener('input', () => {
    if (!$('use-json').checked) $('arguments').value = JSON.stringify(fieldArguments(), null, 2);
});
$('tool-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    if (!activeTool) return;
    const button = $('run-tool');
    button.disabled = true;
    for (const toolButton of $('tool-list').children) toolButton.disabled = true;
    $('call-status').classList.remove('error');
    text('call-status', 'Calling the MCP tool…');
    $('result').hidden = true;
    $('result-title').hidden = true;
    try {
        const args = $('use-json').checked ? JSON.parse($('arguments').value) : fieldArguments();
        if (!args || Array.isArray(args) || typeof args !== 'object')
            throw Error('Arguments must be a JSON object.');
        const request = message('tools/call', { name: activeTool.name, arguments: args });
        text('request', JSON.stringify(request, null, 2));
        $('request-details').hidden = false;
        const result = await rpc(request);
        text('result', JSON.stringify(result.structuredContent ?? result, null, 2));
        $('result').hidden = false;
        $('result-title').hidden = false;
        text(
            'call-status',
            result.isError ? 'Tool returned an error. See the details below.' : 'Tool completed.',
        );
        $('call-status').classList.toggle('error', Boolean(result.isError));
    } catch (error) {
        text('call-status', error.message);
        $('call-status').classList.add('error');
    } finally {
        button.disabled = false;
        for (const toolButton of $('tool-list').children) toolButton.disabled = false;
    }
});
const endpoint = location.origin + '/mcp';
$('endpoint').href = endpoint;
$('endpoint').textContent = endpoint;
void tab(location.hash === '#mcp' ? 'mcp' : 'rest');

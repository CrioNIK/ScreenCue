'use strict';

// Node is a development-only test dependency. ScreenCue itself does not use it.
// Protocol-only by default. Add --live to exercise visible hints on your desktop.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const path = require('node:path');
const explicitExecutable = process.argv.slice(2).find(arg => !arg.startsWith('--'));
const executable = path.resolve(explicitExecutable || path.join(__dirname, '../target/debug/screencue' + (process.platform === 'win32' ? '.exe' : '')));
const liveOverlay = process.argv.includes('--live');

class McpClient {
  constructor() {
    this.child = spawn(executable, ['--mcp'], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    this.queue = [];
    this.waiter = null;
    this.buffer = '';
    this.stderr = '';
    this.failure = null;
    this.id = 0;
    this.closed = new Promise(resolve => this.child.on('close', (code, signal) => resolve({ code, signal })));
    this.child.stdout.setEncoding('utf8');
    this.child.stderr.setEncoding('utf8');
    this.child.stderr.on('data', chunk => { this.stderr += chunk; });
    this.child.on('error', error => this.fail(error));
    this.child.stdin.on('error', error => this.fail(error));
    this.child.stdout.on('data', chunk => {
      this.buffer += chunk;
      let newline;
      while ((newline = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0, newline).replace(/\r$/, '');
        this.buffer = this.buffer.slice(newline + 1);
        try {
          assert.ok(line.length, 'stdout contains an empty non-protocol line');
          assert.notEqual(line.charCodeAt(0), 0xfeff, 'stdout contains a BOM');
          const message = JSON.parse(line);
          assert.equal(message.jsonrpc, '2.0');
          assert.ok(Object.hasOwn(message, 'id'), 'unexpected notification on stdout');
          assert.notEqual(Object.hasOwn(message, 'error'), Object.hasOwn(message, 'result'));
          if (this.waiter) { const waiter = this.waiter; this.waiter = null; waiter.resolve(message); }
          else this.queue.push(message);
        } catch (error) { this.fail(error); }
      }
    });
  }
  fail(error) {
    this.failure = error;
    if (this.waiter) { const waiter = this.waiter; this.waiter = null; waiter.reject(error); }
  }
  next() {
    if (this.failure) return Promise.reject(this.failure);
    if (this.queue.length) return Promise.resolve(this.queue.shift());
    assert.equal(this.waiter, null, 'one test request at a time');
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => { this.waiter = null; reject(new Error('MCP response timeout')); }, 12000);
      this.waiter = {
        resolve(value) { clearTimeout(timeout); resolve(value); },
        reject(error) { clearTimeout(timeout); reject(error); }
      };
    });
  }
  async raw(line) { this.child.stdin.write(line + '\n'); return this.next(); }
  async request(method, params, id = ++this.id) {
    const message = { jsonrpc: '2.0', id, method };
    if (params !== undefined) message.params = params;
    const response = await this.raw(JSON.stringify(message));
    assert.equal(response.id, id, 'response id mismatch or notification received a response');
    return response;
  }
  notify(method, params) { this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method, params }) + '\n'); }
  async tool(name, args = {}) {
    // Live protocol cases keep fixed desktop coordinates; native tracking is
    // tested separately with an owned window in windows-native.ps1.
    if (name === 'screen_hint' && !Object.hasOwn(args, 'follow_window')) args = { ...args, follow_window: false };
    const response = await this.request('tools/call', { name, arguments: args });
    assert.ok(!response.error, JSON.stringify(response));
    assert.equal(response.result.content.length, 1);
    assert.equal(response.result.content[0].type, 'text');
    return { result: response.result, data: JSON.parse(response.result.content[0].text) };
  }
  async finish(expectedExit = 0) {
    this.child.stdin.end();
    const timer = setTimeout(() => this.child.kill(), 12000);
    const ended = await this.closed;
    clearTimeout(timer);
    assert.equal(ended.signal, null, 'MCP did not exit after stdin EOF');
    assert.equal(ended.code, expectedExit);
    if (this.failure) throw this.failure;
    assert.equal(this.queue.length, 0, 'unconsumed output / notification response');
    assert.equal(this.buffer, '', 'unterminated stdout data');
    assert.equal(this.stderr, '', 'unexpected MCP diagnostics');
  }
  stop() { this.child.kill(); }
}

async function initialize(client, version = '2025-11-25') {
  const reply = await client.request('initialize', { protocolVersion: version, capabilities: {}, clientInfo: { name: 'screencue-smoke', version: '1' } });
  assert.ok(reply.result, JSON.stringify(reply));
  assert.equal(reply.result.protocolVersion, version);
  assert.equal(reply.result.serverInfo.name, 'screencue');
  assert.ok(reply.result.capabilities.tools);
  assert.match(reply.result.instructions, /physical desktop pixels/);
  client.notify('notifications/initialized', {});
}

async function main() {
  let client = new McpClient();
  try {
    assert.equal((await client.request('tools/list')).error.code, -32000);
    await initialize(client);
    client.notify('notifications/cancelled', { requestId: 900 });
    client.notify('notifications/unknown', {});
    assert.deepEqual((await client.request('ping')).result, {});
    const listed = (await client.request('tools/list')).result.tools;
    assert.deepEqual(listed.map(tool => tool.name).sort(), ['screen_clear', 'screen_displays', 'screen_hint', 'screen_prompt', 'screen_status']);
    const hintSchema = listed.find(tool => tool.name === 'screen_hint').inputSchema;
    assert.equal(hintSchema.properties.duration_ms.default, 0);
    assert.equal(hintSchema.properties.follow_window.default, true);
    assert.equal(hintSchema.properties.follow_window.type, 'boolean');
    assert.equal(hintSchema.additionalProperties, false);
    assert.equal(hintSchema.properties.color.default, 'auto');
    assert.equal(hintSchema.properties.arrow.type, 'boolean');
    assert.equal(hintSchema.properties.target.default, true);
    assert.deepEqual(hintSchema.properties.shape.enum, ['ellipse', 'rectangle', 'rounded', 'polygon']);
    assert.equal(hintSchema.properties.points.maxItems, 64);
    assert.equal(hintSchema.properties.points.minItems, 3);
    assert.equal(hintSchema.properties.title.default, '');
    assert.equal(hintSchema.properties.title.maxLength, 100);
    assert.equal(hintSchema.properties.text.maxLength, 16000);
    assert.equal(hintSchema.properties.text_page.default, 0);
    assert.equal(hintSchema.properties.panel_x.type, 'integer');
    assert.equal(hintSchema.properties.panel_width.minimum, 120);
    for (const arguments_ of [
      { x: 0, y: 0, title: 'x'.repeat(101) },
      { x: 0, y: 0, text: 'x'.repeat(16001) },
      { x: 0, y: 0, panel_x: 20 },
      { x: 0, y: 0, text_page: -1 }
    ]) {
      assert.equal((await client.tool('screen_hint', arguments_)).result.isError, true);
    }
    assert.equal((await client.request('unknown/method')).error.code, -32601);
    assert.equal((await client.request('tools/call', { name: 'unknown_tool' })).error.code, -32602);
    assert.equal((await client.request('tools/list', [])).error.code, -32602);
    for (const params of [
      { name: 'screen_hint', arguments: [] },
      { name: 'screen_hint', arguments: null },
      { name: 'screen_hint', arguments: { command: 'quit' } },
      { name: 'screen_status', arguments: { extra: true } },
      { name: 'screen_clear', arguments: { extra: true } }
    ]) {
      const reply = await client.request('tools/call', params);
      assert.equal(reply.error?.code, -32602, 'invalid tool envelope accepted: ' + JSON.stringify(params));
    }
    for (const line of [
      '{', "{'jsonrpc':'2.0','id':1,'method':'ping'}",
      '{jsonrpc:"2.0","id":1,"method":"ping"}',
      '{"jsonrpc":"2.0","id":1,"method":"ping",}',
      '{"jsonrpc":"2.0","id":01,"method":"ping"}',
      '{"jsonrpc":"2.0","id":1,"method":"ping"} garbage'
    ]) {
      const reply = await client.raw(line);
      assert.equal(reply.id, null);
      assert.equal(reply.error.code, -32700, 'accepted non-JSON input: ' + line);
    }
    for (const value of [[], [1], 1, 'text', null, true, {}, { jsonrpc: '1.0', id: 3, method: 'ping' }]) {
      const reply = await client.raw(JSON.stringify(value));
      assert.equal(reply.error.code, -32600, 'expected invalid-request error: ' + JSON.stringify(value));
    }
    assert.deepEqual((await client.request('ping', undefined, 'request-世界')).result, {});

    if (liveOverlay) {
      const { data: geometry } = await client.tool('screen_displays');
      assert.equal(geometry.ok, true);
      assert.ok(geometry.displays.length > 0);
      const monitor = geometry.displays.find(display => display.primary) || geometry.displays[0];
      const x = monitor.x + Math.floor(monitor.width / 2);
      const y = monitor.y + Math.floor(monitor.height / 2);
      const text = 'Open «Settings» — 世界 😀\nNext step';
      const shown = await client.tool('screen_hint', { x, y, text });
      assert.equal(shown.result.isError, false);
      assert.equal(shown.data.visible, true);
      const { data: status } = await client.tool('screen_status');
      assert.equal(status.hint.text, text);
      assert.equal(status.hint.duration_ms, 0);
      const offscreen = await client.tool('screen_hint', { x: -100000, y: -100000, text: 'Outside' });
      assert.equal(offscreen.result.isError, true);
      assert.equal(offscreen.data.ok, false);
      assert.equal((await client.tool('screen_status')).data.hint.text, text, 'failed hint replaced the previous hint');
      for (const shape of ['ellipse', 'rectangle', 'rounded']) {
        const cue = await client.tool('screen_hint', { x, y, text: 'Marker only', shape, width: 60, height: 40, arrow: false, target: true, color: '#4F8CFF' });
        assert.equal(cue.result.isError, false, JSON.stringify(cue));
        assert.equal(cue.data.visible, true);
      }
      const polygon = await client.tool('screen_hint', {
        x, y, text: 'Polygon', shape: 'polygon', color: '#fF8844', arrow: true, target: true,
        points: [{ x: x - 30, y: y - 20 }, { x: x + 30, y: y - 20 }, { x, y: y + 40 }]
      });
      assert.equal(polygon.result.isError, false, JSON.stringify(polygon));
      const arrowOnly = await client.tool('screen_hint', { x, y, text: 'Arrow only', arrow: true, target: false, color: '#00FF88' });
      assert.equal(arrowOnly.result.isError, false, JSON.stringify(arrowOnly));
      const longText = 'A readable instruction with details, examples, and a separate next step.\n'.repeat(150);
      const panelRequest = { x, y, title: 'Independent text panel', text: longText,
        arrow: false, target: false, follow_window: false, color: '#EF232D',
        panel_x: monitor.x + 20, panel_y: monitor.y + 20, panel_width: 320 };
      const panel = await client.tool('screen_hint', panelRequest);
      assert.equal(panel.result.isError, false, JSON.stringify(panel));
      assert.equal(panel.data.arrow_tip, null);
      assert.equal(panel.data.panel.page, 0);
      assert.ok(panel.data.panel.page_count > 1);
      assert.equal(panel.data.panel.x, monitor.x + 20);
      const pageTwo = await client.tool('screen_hint', { ...panelRequest, text_page: 1 });
      assert.equal(pageTwo.data.panel.page, 1);
      assert.equal(pageTwo.data.panel.page_count, panel.data.panel.page_count);
      assert.equal((await client.tool('screen_status')).data.hint.text, longText);
      const titleOnly = await client.tool('screen_hint', { x, y, title: 'Heading only', text: '', arrow: false, target: false });
      assert.equal(titleOnly.result.isError, false);
      assert.ok(titleOnly.data.panel.width > 0);
      const prompt = await client.tool('screen_prompt', {title: 'Conversation', text: 'Ask which milestone matters most.'});
      assert.equal(prompt.result.isError, false, JSON.stringify(prompt));
      assert.equal(prompt.data.arrow_tip, null);
      assert.ok(prompt.data.panel.width > 0);
      assert.equal(prompt.data.capture_privacy.capture_guaranteed, false);
      const promptStatus = (await client.tool('screen_status')).data;
      assert.equal(promptStatus.hint.target, false);
      assert.equal(promptStatus.hint.follow_window, false);
      assert.equal((await client.tool('screen_prompt', {text: 'Invalid', arrow:true})).result.isError, true);
      const cleared = await client.tool('screen_clear');
      assert.equal(cleared.result.isError, false);
      assert.equal((await client.tool('screen_status')).data.visible, false);
    }
    await client.finish();
    client = new McpClient();
    await initialize(client, '2024-11-05');
    await client.finish();
    client = new McpClient();
    const oversized = await client.raw(' '.repeat(131073) + '\n' + JSON.stringify({ jsonrpc: '2.0', id: 999, method: 'ping' }));
    assert.equal(oversized.error.code, -32700);
    await client.finish();
    console.log('PASS: MCP lifecycle, notifications, schema, validation, strict JSON, Unicode and EOF' + (liveOverlay ? ', with live hint tools.' : ' (protocol-only).'));
  } finally { client.stop(); }
}

main().catch(error => { console.error(error.stack || error); process.exitCode = 1; });

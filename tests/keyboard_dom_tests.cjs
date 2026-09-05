// Run with: node --test tests/keyboard_dom_tests.cjs
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const keyboardSource = fs.readFileSync(path.join(__dirname, '../src/keyboard/input.js'), 'utf8');

class InputEvent {
    constructor(type, options) { Object.assign(this, options, { type }); }
}

class Field {
    constructor(tagName, value, type = 'text') {
        Object.assign(this, {
            tagName, type, storedValue: value, selectionStart: value.length,
            selectionEnd: value.length, isConnected: true, maxLength: -1,
            events: [], disabled: false, readOnly: false, isContentEditable: false,
            focusCalls: 0
        });
    }
    get value() { return this.storedValue; }
    set value(value) { this.storedValue = value; }
    focus() {
        this.focusCalls++;
        this.ownerDocument.activeElement = this;
    }
    setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; }
    dispatchEvent(event) {
        this.events.push(event);
        if (this.onEvent) this.onEvent(event);
        return event.type !== this.cancelEvent;
    }
}

function keyboard(field, lastInput = null) {
    const document = {
        activeElement: field,
        defaultView: { HTMLInputElement: Field, HTMLTextAreaElement: Field, InputEvent, KeyboardEvent: InputEvent }
    };
    if (field) field.ownerDocument = document;
    if (lastInput) lastInput.ownerDocument = document;
    const context = vm.createContext({ document, window: { __safebrowse_last_input: lastInput }, Intl });
    vm.runInContext(keyboardSource, context);
    return (action) => context.applyVirtualKey(action);
}

test('password selection replacement updates framework state through native input setter', () => {
    const field = new Field('INPUT', 'secret', 'password');
    let frameworkValue = field.value;
    Object.defineProperty(field, 'value', {
        get() { return this.storedValue; },
        set(value) { throw new Error(`Framework setter should not intercept ${value}`); }
    });
    field.onEvent = (event) => { if (event.type === 'input') frameworkValue = field.value; };
    field.setSelectionRange(1, 5);
    keyboard(field)('X');
    assert.equal(field.value, 'sXt');
    assert.equal(frameworkValue, 'sXt');
    assert.equal(field.selectionStart, 2);
    assert.deepEqual(field.events.map((event) => event.type), ['beforeinput', 'input']);
});

test('backspace deletes a selected range or an entire Unicode grapheme', () => {
    const field = new Field('INPUT', 'a👩‍💻b');
    field.setSelectionRange(field.value.length - 1, field.value.length - 1);
    const press = keyboard(field);
    press('BACKSPACE');
    assert.equal(field.value, 'ab');
    assert.equal(field.selectionStart, 1);
    field.setSelectionRange(0, 2);
    press('BACKSPACE');
    assert.equal(field.value, '');
});

test('disabled, readonly, disconnected, and absent inputs are not edited', () => {
    for (const property of ['disabled', 'readOnly', 'isConnected']) {
        const field = new Field('INPUT', 'unchanged');
        field[property] = property !== 'isConnected';
        keyboard(field)('X');
        assert.equal(field.value, 'unchanged');
        assert.equal(field.events.length, 0);
    }
    keyboard(null)('X');
});

test('maxlength and canceled beforeinput do not corrupt the value', () => {
    const field = new Field('INPUT', 'ab');
    field.maxLength = 3;
    const press = keyboard(field);
    press('😀');
    assert.equal(field.value, 'ab');
    field.cancelEvent = 'beforeinput';
    press('c');
    assert.equal(field.value, 'ab');
    assert.equal(field.events.filter((event) => event.type === 'input').length, 0);
});

test('textarea Enter inserts a newline; canceled input Enter does not submit', () => {
    const textarea = new Field('TEXTAREA', 'line');
    textarea.form = { requestSubmit() { throw new Error('Textarea Enter must not submit'); } };
    keyboard(textarea)('ENTER');
    assert.equal(textarea.value, 'line\n');

    const input = new Field('INPUT', 'value');
    let submissions = 0;
    input.form = { requestSubmit() { submissions++; } };
    const press = keyboard(input);
    input.cancelEvent = 'keydown';
    press('ENTER');
    assert.equal(submissions, 0);
    input.cancelEvent = null;
    press('ENTER');
    assert.equal(submissions, 1);
});

test('a remembered focused field accepts punctuation as text after shell focus', () => {
    const field = new Field('INPUT', '');
    const action = "'; alert(1); '\n\"\\";
    keyboard(null, field)(action);
    assert.equal(field.value, action);
});

test('a delayed virtual key preserves a different control focus and the remembered caret', () => {
    const minimizeControl = new Field('BUTTON', '');
    const field = new Field('INPUT', 'before after');
    field.setSelectionRange(7, 7);
    const press = keyboard(minimizeControl, field);

    press('X');
    assert.equal(field.value, 'before Xafter');
    assert.equal(field.selectionStart, 8);
    assert.equal(field.ownerDocument.activeElement, minimizeControl);
    assert.equal(field.focusCalls, 0);
    assert.deepEqual(field.events.map((event) => event.type), ['beforeinput', 'input']);

    press('BACKSPACE');
    assert.equal(field.value, 'before after');
    assert.equal(field.selectionStart, 7);
    assert.equal(field.ownerDocument.activeElement, minimizeControl);
    assert.equal(field.focusCalls, 0);
});

test('textarea Enter after focus leaves the document inserts at its stored caret without refocusing', () => {
    const textarea = new Field('TEXTAREA', 'ab');
    textarea.setSelectionRange(1, 1);
    keyboard(null, textarea)('ENTER');

    assert.equal(textarea.value, 'a\nb');
    assert.equal(textarea.selectionStart, 2);
    assert.equal(textarea.ownerDocument.activeElement, null);
    assert.equal(textarea.focusCalls, 0);
});

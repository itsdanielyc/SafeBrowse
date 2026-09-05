// Run with: node --test tests/website_print_guard_tests.cjs
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const guardSource = fs.readFileSync(path.join(__dirname, '../src/browser/printing/website_print_guard.js'), 'utf8');

class FixtureEventTarget {
    constructor() { this.listeners = new Map(); }
    addEventListener(type, callback, options = {}) {
        const entries = this.listeners.get(type) || [];
        entries.push({ callback, once: Boolean(options.once) });
        this.listeners.set(type, entries);
    }
    dispatch(type) {
        const event = { defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
        for (const entry of [...(this.listeners.get(type) || [])]) {
            if (entry.once) {
                this.listeners.set(type, this.listeners.get(type).filter(candidate => candidate !== entry));
            }
            entry.callback(event);
        }
        return event;
    }
}

class FixtureElement extends FixtureEventTarget {
    constructor(tagName, ownerDocument) {
        super();
        Object.assign(this, { tagName, ownerDocument, children: [], attributes: new Map(), style: {}, parent: null });
    }
    get isConnected() { return this === this.ownerDocument.body || Boolean(this.parent?.isConnected); }
    setAttribute(name, value) { this.attributes.set(name, value); }
    append(...elements) {
        for (const element of elements) {
            element.parent = this;
            this.children.push(element);
        }
    }
    remove() {
        if (this.parent) this.parent.children = this.parent.children.filter(child => child !== this);
        this.parent = null;
    }
    focus() { throw new Error('The print notice must not request focus'); }
}

function makeFixture({ early = false, locked = false } = {}) {
    const document = new FixtureEventTarget();
    document.readyState = early ? 'loading' : 'complete';
    document.createElement = tagName => new FixtureElement(tagName, document);
    document.body = early ? null : document.createElement('body');
    const focusedInput = { value: 'unmodified', selectionStart: 2, selectionEnd: 5 };
    document.activeElement = focusedInput;
    let nativeCalls = 0;
    const window = {};
    Object.defineProperty(window, 'print', {
        value() { nativeCalls++; }, configurable: !locked, writable: !locked
    });
    const context = vm.createContext({ window, document });
    vm.runInContext(guardSource, context);
    return { document, window, context, focusedInput, nativeCalls: () => nativeCalls };
}

test('ordinary print calls show one accessible notice without native printing or focus changes', () => {
    const fixture = makeFixture();
    for (let index = 0; index < 100; index++) fixture.window.print();
    assert.equal(fixture.nativeCalls(), 0);
    assert.equal(fixture.document.body.children.length, 1);
    const notice = fixture.document.body.children[0];
    assert.equal(notice.attributes.get('role'), 'status');
    assert.equal(notice.attributes.get('aria-live'), 'polite');
    assert.equal(notice.children[0].textContent, 'Website print dialogs are suppressed. To print, use SafeBrowse’s Print button or Ctrl+P. If printing is disabled, enable it in Settings.');
    assert.equal(fixture.document.activeElement, fixture.focusedInput);
    assert.deepEqual(fixture.focusedInput, { value: 'unmodified', selectionStart: 2, selectionEnd: 5 });
});

test('strict assignment cannot replace print and redefinition is rejected', () => {
    const fixture = makeFixture();
    const suppressedPrint = fixture.window.print;
    vm.runInContext('"use strict"; window.print = () => { throw new Error("replacement ran"); }; window.print();', fixture.context);
    assert.equal(fixture.window.print, suppressedPrint);
    assert.equal(fixture.nativeCalls(), 0);
    assert.throws(() => vm.runInContext('Object.defineProperty(window, "print", {value() {}})', fixture.context), /redefine/);
    assert.throws(() => vm.runInContext('"use strict"; delete window.print;', fixture.context), /delete/);
    assert.equal(Object.getOwnPropertyDescriptor(fixture.window, 'print').configurable, false);
    vm.runInContext(guardSource, fixture.context);
    assert.equal(fixture.window.print, suppressedPrint, 'reinjection must retain the first bounded guard');
});

test('early repeated calls schedule one notice and release their listener when the DOM arrives', () => {
    const fixture = makeFixture({ early: true });
    for (let index = 0; index < 100; index++) fixture.window.print();
    assert.equal(fixture.document.listeners.get('DOMContentLoaded').length, 1);
    assert.equal(fixture.nativeCalls(), 0);
    fixture.document.body = fixture.document.createElement('body');
    fixture.document.readyState = 'interactive';
    fixture.document.dispatch('DOMContentLoaded');
    assert.equal(fixture.document.body.children.length, 1);
    assert.equal(fixture.document.listeners.get('DOMContentLoaded').length, 0);
    fixture.window.print();
    assert.equal(fixture.document.body.children.length, 1);
    assert.equal(fixture.document.activeElement, fixture.focusedInput);
});

test('dismissal preserves editable focus and a later print request creates only one new notice', () => {
    const fixture = makeFixture();
    fixture.window.print();
    const dismiss = fixture.document.body.children[0].children[1];
    assert.equal(dismiss.attributes.get('aria-label'), 'Dismiss print notice');
    assert.equal(dismiss.dispatch('pointerdown').defaultPrevented, true);
    dismiss.dispatch('click');
    assert.equal(fixture.document.body.children.length, 0);
    assert.equal(fixture.document.activeElement, fixture.focusedInput);
    fixture.window.print();
    dismiss.dispatch('click');
    fixture.window.print();
    assert.equal(fixture.document.body.children.length, 1);
    assert.equal(fixture.nativeCalls(), 0);
});

test('a reentrant page DOM callback cannot recursively allocate duplicate notices', () => {
    const fixture = makeFixture();
    const originalAppend = fixture.document.body.append.bind(fixture.document.body);
    fixture.document.body.append = (...elements) => {
        fixture.window.print();
        originalAppend(...elements);
    };
    assert.doesNotThrow(() => fixture.window.print());
    assert.equal(fixture.document.body.children.length, 1);
    assert.equal(fixture.nativeCalls(), 0);
});

test('DOM teardown and preexisting nonconfigurable properties never trigger fallback printing', () => {
    const destroyed = makeFixture();
    destroyed.document.createElement = () => { throw new Error('Document is closing'); };
    assert.doesNotThrow(() => destroyed.window.print());
    assert.equal(destroyed.nativeCalls(), 0);
    const early = makeFixture({ early: true });
    early.window.print();
    Object.defineProperty(early.document, 'body', { get() { throw new Error('Detached document'); } });
    assert.doesNotThrow(() => early.document.dispatch('DOMContentLoaded'));
    assert.equal(early.nativeCalls(), 0);
    const locked = makeFixture({ locked: true });
    assert.equal(locked.nativeCalls(), 0, 'failed installation must not invoke the existing property');
});

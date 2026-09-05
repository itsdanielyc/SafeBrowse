function applyVirtualKey(action) {
    const editableInputTypes = new Set(['text', 'search', 'url', 'tel', 'email', 'password', 'number']);

    function isEditable(element) {
        if (!element || !element.isConnected || element.disabled || element.readOnly) return false;
        return element.tagName === 'TEXTAREA'
            || (element.tagName === 'INPUT' && editableInputTypes.has(element.type))
            || element.isContentEditable;
    }

    function focusedElement() {
        let element = document.activeElement;
        while (element && element.shadowRoot && element.shadowRoot.activeElement) {
            element = element.shadowRoot.activeElement;
        }
        return element;
    }

    const focused = focusedElement();
    // Never guess a destination: pages often contain hidden or unrelated fields.
    const el = isEditable(focused) ? focused : window.__safebrowse_last_input;
    if (!isEditable(el) || typeof action !== 'string' || action.length === 0) return;

    const ownerDocument = el.ownerDocument;
    const view = ownerDocument.defaultView;
    // Native script evaluation is asynchronous. Refocusing here can take focus
    // back from a shell control the user clicked after the virtual key.
    // Stored field selections and contenteditable ranges can be edited in place.

    function beforeInput(inputType, data) {
        return el.dispatchEvent(new view.InputEvent('beforeinput', {
            bubbles: true, composed: true, cancelable: true, inputType, data
        }));
    }

    function deleteStart(value, caret) {
        if (caret === 0) return 0;
        const prefix = value.slice(0, caret);
        if (typeof Intl.Segmenter === 'function') {
            let start = 0;
            for (const segment of new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(prefix)) {
                start = segment.index;
            }
            return start;
        }
        const previousCharacter = Array.from(prefix).pop();
        return caret - previousCharacter.length;
    }

    function replaceFieldSelection(text, deleting) {
        const value = el.value;
        let start = typeof el.selectionStart === 'number' ? el.selectionStart : value.length;
        const end = typeof el.selectionEnd === 'number' ? el.selectionEnd : value.length;
        if (deleting && start === end) start = deleteStart(value, start);

        if (!deleting && el.maxLength >= 0) {
            const remaining = Math.max(0, el.maxLength - (value.length - (end - start)));
            // Avoid introducing an isolated surrogate when maxlength splits emoji.
            text = text.slice(0, remaining).replace(/[\uD800-\uDBFF]$/, '');
        }
        const updatedValue = value.slice(0, start) + text + value.slice(end);
        if (updatedValue === value) return;

        const inputType = deleting ? 'deleteContentBackward' : 'insertText';
        const data = deleting ? null : text;
        if (!beforeInput(inputType, data)) return;

        // Frameworks such as React track own-property setters. The browser's native
        // setter plus an input event updates both the DOM and controlled form state.
        const prototype = el.tagName === 'TEXTAREA'
            ? view.HTMLTextAreaElement.prototype : view.HTMLInputElement.prototype;
        const setter = Object.getOwnPropertyDescriptor(prototype, 'value').set;
        setter.call(el, updatedValue);
        // Number inputs sanitize unsupported intermediate strings to empty. A
        // virtual key must not erase an existing amount when that happens.
        if (el.type === 'number' && updatedValue !== '' && el.value === '') {
            setter.call(el, value);
            return;
        }
        if (typeof el.selectionStart === 'number') {
            el.setSelectionRange(start + text.length, start + text.length);
        }
        el.dispatchEvent(new view.InputEvent('input', {
            bubbles: true, composed: true, inputType, data
        }));
    }

    /** O(n) time and space over the text before the caret, including split text nodes. */
    function extendRangeForBackspace(range) {
        if (!range.collapsed) return true;
        let previousNode = range.startContainer;
        if (previousNode.nodeType === view.Node.ELEMENT_NODE && range.startOffset > 0) {
            previousNode = previousNode.childNodes[range.startOffset - 1];
        } else if (range.startOffset === 0) {
            while (previousNode !== el && !previousNode.previousSibling) previousNode = previousNode.parentNode;
            previousNode = previousNode === el ? null : previousNode.previousSibling;
        } else {
            previousNode = null;
        }
        while (previousNode) {
            if (previousNode.lastChild) {
                previousNode = previousNode.lastChild;
                continue;
            }
            if (previousNode.nodeType !== view.Node.TEXT_NODE || previousNode.length > 0) break;
            // Range insertion and deletion can leave empty text nodes between lines.
            while (previousNode !== el && !previousNode.previousSibling) previousNode = previousNode.parentNode;
            previousNode = previousNode === el ? null : previousNode.previousSibling;
        }
        if (previousNode?.nodeName === 'BR') {
            range.setStartBefore(previousNode);
            return true;
        }

        const prefixRange = ownerDocument.createRange();
        prefixRange.selectNodeContents(el);
        prefixRange.setEnd(range.startContainer, range.startOffset);
        const prefix = prefixRange.toString();
        if (!prefix.length) return false;
        let remainingOffset = deleteStart(prefix, prefix.length);
        const walker = ownerDocument.createTreeWalker(el, view.NodeFilter.SHOW_TEXT);
        let textNode = walker.nextNode();
        while (textNode) {
            if (remainingOffset < textNode.length) {
                range.setStart(textNode, remainingOffset);
                return true;
            }
            remainingOffset -= textNode.length;
            textNode = walker.nextNode();
        }
        return false;
    }

    function editContent(text, deleting, lineBreak = false) {
        const selection = ownerDocument.getSelection();
        if (!selection || selection.rangeCount === 0 || !el.contains(selection.anchorNode)) return;
        const range = selection.getRangeAt(0);
        if (!el.contains(range.startContainer) || !el.contains(range.endContainer)) return;
        const ownsFocus = ownerDocument.hasFocus() && focusedElement() === el;
        const inputType = lineBreak ? (ownsFocus ? 'insertParagraph' : 'insertLineBreak')
            : deleting ? 'deleteContentBackward' : 'insertText';
        const data = deleting || lineBreak ? null : text;
        if (!beforeInput(inputType, data)) return;
        if (!isEditable(el) || !el.contains(range.startContainer) || !el.contains(range.endContainer)) return;
        if (ownsFocus && (!ownerDocument.hasFocus() || focusedElement() !== el)) return;
        if (ownsFocus) {
            // Browser commands retain native undo when the editor already owns focus.
            const command = lineBreak ? 'insertParagraph' : deleting ? 'delete' : 'insertText';
            ownerDocument.execCommand(command, false, data);
            return;
        }

        // Chromium's editing commands refocus an inactive editing host. Mutating
        // the existing live range avoids that activation while retaining its caret.
        // These direct edits cannot participate in Chromium's native undo history.
        if (deleting && !extendRangeForBackspace(range)) return;
        range.deleteContents();
        if (lineBreak) {
            const insertion = ownerDocument.createElement('br');
            range.insertNode(insertion);
            // A terminal break needs a following line box to display its caret.
            if (!insertion.nextSibling) insertion.after(ownerDocument.createElement('br'));
            range.setStartAfter(insertion);
        } else if (text) {
            const insertion = ownerDocument.createTextNode(text);
            range.insertNode(insertion);
            range.setStart(insertion, text.length);
        }
        range.collapse(true);
        el.dispatchEvent(new view.InputEvent('input', {
            bubbles: true, composed: true, inputType, data
        }));
    }

    if (action === 'ENTER') {
        const allowed = el.dispatchEvent(new view.KeyboardEvent('keydown', {
            key: 'Enter', code: 'Enter', bubbles: true, composed: true, cancelable: true
        }));
        if (allowed) {
            if (el.tagName === 'TEXTAREA') {
                replaceFieldSelection('\n', false);
            } else if (el.isContentEditable) {
                editContent('', false, true);
            } else if (el.form && typeof el.form.requestSubmit === 'function') {
                el.form.requestSubmit();
            }
        }
        el.dispatchEvent(new view.KeyboardEvent('keyup', {
            key: 'Enter', code: 'Enter', bubbles: true, composed: true
        }));
        return;
    }

    const deleting = action === 'BACKSPACE';
    if (el.isContentEditable) {
        editContent(deleting ? '' : action, deleting);
    } else {
        replaceFieldSelection(deleting ? '' : action, deleting);
    }
}

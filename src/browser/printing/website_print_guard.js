(() => {
    'use strict';

    const NOTICE_TEXT = 'Website print dialogs are suppressed. To print, use SafeBrowse’s Print button or Ctrl+P. If printing is disabled, enable it in Settings.';
    const NOTICE_LABEL = 'Website printing';
    const DISMISS_LABEL = 'Dismiss print notice';
    const NOTICE_Z_INDEX = 2147483647;
    let notice = null;
    let waitingForDocument = false;
    let renderingNotice = false;

    /** Keeps only one notice per document without moving the current input focus. */
    function showSuppressionNotice() {
        if (renderingNotice) return;
        renderingNotice = true;
        try {
            if (notice?.isConnected) return;
            if (!document.body) {
                if (!waitingForDocument && document.readyState === 'loading') {
                    waitingForDocument = true;
                    document.addEventListener('DOMContentLoaded', () => {
                        waitingForDocument = false;
                        showSuppressionNotice();
                    }, { once: true });
                }
                return;
            }

            const container = document.createElement('div');
            container.setAttribute('role', 'status');
            container.setAttribute('aria-live', 'polite');
            container.setAttribute('aria-atomic', 'true');
            container.setAttribute('aria-label', NOTICE_LABEL);
            container.style.cssText = `position:fixed;right:16px;bottom:16px;z-index:${NOTICE_Z_INDEX};box-sizing:border-box;width:420px;max-width:calc(100vw - 32px);display:flex;align-items:flex-start;gap:12px;padding:12px 14px;border:1px solid #c8c8c8;border-radius:6px;background:#fff;color:#333;box-shadow:0 3px 14px #0002;font:12px/1.45 "Segoe UI",sans-serif;text-align:left;`;
            const message = document.createElement('span');
            message.textContent = NOTICE_TEXT;
            const dismiss = document.createElement('button');
            dismiss.type = 'button';
            dismiss.textContent = 'Dismiss';
            dismiss.setAttribute('aria-label', DISMISS_LABEL);
            dismiss.style.cssText = 'flex-shrink:0;padding:3px 7px;border:1px solid #bbb;border-radius:4px;background:#f5f5f5;color:#333;font:inherit;cursor:pointer;';
            dismiss.addEventListener('pointerdown', event => {
                // Pointer dismissal must preserve a field's focus and selection.
                try { event.preventDefault(); } catch (_) {}
            });
            dismiss.addEventListener('click', () => {
                try { container.remove(); } catch (_) {}
                if (notice === container) notice = null;
            });
            container.append(message, dismiss);
            document.body.append(container);
            notice = container;
        } catch (_) {
            // Document teardown or site DOM changes must never restore native printing or throw into a page.
        } finally {
            renderingNotice = false;
        }
    }

    try {
        Object.defineProperty(window, 'print', {
            configurable: false,
            enumerable: true,
            get() { return showSuppressionNotice; },
            // Compatibility libraries sometimes assign window.print in strict mode.
            set(_) {}
        });
    } catch (_) {
        // A nonconfigurable preexisting property cannot be replaced; there is no native fallback here.
    }
})();

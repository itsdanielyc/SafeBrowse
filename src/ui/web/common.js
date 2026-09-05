'use strict';

/** Sends one structured command to the native shell, when hosted by SafeBrowse. */
function postIpc(message) {
    if (window.ipc && typeof window.ipc.postMessage === 'function') {
        window.ipc.postMessage(JSON.stringify(message));
    }
}

/** Attaches commands to semantic buttons; one click produces exactly one command. */
function bindCommandButtons() {
    document.querySelectorAll('button[data-command]').forEach(button => {
        button.addEventListener('click', () => postIpc({ type: button.dataset.command }));
    });
}

/** Notifies the shell only after every callback and element is ready to receive updates. */
function notifyReady() {
    postIpc({ type: 'UI_READY' });
}

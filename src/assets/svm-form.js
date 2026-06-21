document.addEventListener('submit', function(e) {
    var form = e.target;
    if (!form.hasAttribute('data-svm-form')) return;
    e.preventDefault();

    var url = form.getAttribute('action');
    var json = {};

    form.querySelectorAll('[data-svm-field]').forEach(function(el) {
        var keys = el.name.split('.');
        var obj = json;
        for (var i = 0; i < keys.length - 1; i++) {
            if (!(keys[i] in obj)) obj[keys[i]] = {};
            obj = obj[keys[i]];
        }
        var key = keys[keys.length - 1];
        if (el.type === 'checkbox') {
            obj[key] = el.checked;
        } else if (el.type === 'number') {
            var val = el.value;
            if (val === '') {
                obj[key] = 0;
            } else if (el.step && el.step !== '1' && el.step !== 'any') {
                obj[key] = parseFloat(val);
            } else if (val.indexOf('.') !== -1) {
                obj[key] = parseFloat(val);
            } else {
                obj[key] = parseInt(val, 10);
            }
        } else {
            obj[key] = el.value;
        }
    });

    var csrfEl = form.querySelector('[name="csrf_token"]');
    if (csrfEl) json.csrf_token = csrfEl.value;

    fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(json)
    }).then(function(res) {
        if (res.ok) {
            window.location.reload();
        } else {
            res.text().then(function(t) { alert('Save failed: ' + t); });
        }
    }).catch(function(err) {
        alert('Save failed: ' + err);
    });
});

(function() {
    var input = document.getElementById('svm-field-search');
    if (!input) return;

    var countEl = document.getElementById('svm-search-count');
    var fields = document.querySelectorAll('[data-svm-searchable]');
    var total = fields.length;

    input.addEventListener('input', function() {
        var q = this.value.toLowerCase().trim();
        if (!q) {
            fields.forEach(function(el) { el.style.display = ''; });
            document.querySelectorAll('.svm-fieldset').forEach(function(fs) { fs.style.display = ''; });
            document.querySelectorAll('.svm-subgroup-title').forEach(function(h) { h.style.display = ''; });
            if (countEl) countEl.textContent = '';
            return;
        }

        var shown = 0;
        fields.forEach(function(el) {
            var text = el.getAttribute('data-svm-searchable').toLowerCase();
            var match = text.indexOf(q) !== -1;
            el.style.display = match ? '' : 'none';
            if (match) shown++;
        });

        // Hide fieldsets where all children are hidden
        document.querySelectorAll('.svm-fieldset').forEach(function(fs) {
            var visible = fs.querySelectorAll('.svm-field:not([style*="display: none"])');
            fs.style.display = visible.length ? '' : 'none';
        });
        // Hide subgroup titles where next sibling fields are all hidden
        document.querySelectorAll('.svm-subgroup-title').forEach(function(h) {
            var next = h.nextElementSibling;
            var anyVisible = false;
            while (next && !next.classList.contains('svm-subgroup-title')) {
                if (next.classList.contains('svm-field') && next.style.display !== 'none') {
                    anyVisible = true;
                    break;
                }
                next = next.nextElementSibling;
            }
            h.style.display = anyVisible ? '' : 'none';
        });

        if (countEl) {
            countEl.textContent = shown + ' of ' + total + ' fields';
        }
    });
})();

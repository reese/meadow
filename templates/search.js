(function () {
  const input = document.getElementById('search');
  const box = document.getElementById('search-results');
  if (!input || !box) return;
  let index = [];
  let active = -1;
  fetch('search.json').then(r => r.json()).then(data => { index = data; }).catch(() => {});
  function render(results) {
    box.innerHTML = '';
    active = -1;
    if (!results.length) { box.style.display = 'none'; return; }
    results.slice(0, 20).forEach((item, i) => {
      const a = document.createElement('a');
      a.href = item.url;
      a.textContent = item.path + ' — ' + item.kind;
      a.dataset.index = String(i);
      box.appendChild(a);
    });
    box.style.display = 'block';
  }
  input.addEventListener('input', () => {
    const q = input.value.trim().toLowerCase();
    if (!q) { box.style.display = 'none'; return; }
    const results = index.filter(item => item.path.toLowerCase().includes(q) || (item.name || '').toLowerCase().includes(q));
    render(results);
  });
  input.addEventListener('keydown', event => {
    const links = Array.from(box.querySelectorAll('a'));
    if (!links.length) return;
    if (event.key === 'ArrowDown') { active = Math.min(active + 1, links.length - 1); }
    else if (event.key === 'ArrowUp') { active = Math.max(active - 1, 0); }
    else if (event.key === 'Enter' && active >= 0) { links[active].click(); return; }
    else return;
    event.preventDefault();
    links.forEach((link, i) => link.classList.toggle('active', i === active));
    links[active].scrollIntoView({ block: 'nearest' });
  });
})();

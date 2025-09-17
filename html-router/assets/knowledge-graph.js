// Knowledge graph renderer: interactive 2D force graph with
// zoom/pan, search, neighbor highlighting, curved links with arrows,
// responsive resize, and type/relationship legends.
(function () {
  const D3_SRC = '/assets/d3.min.js';

  let d3Loading = null;

  function ensureD3() {
    if (window.d3) return Promise.resolve();
    if (d3Loading) return d3Loading;
    d3Loading = new Promise((resolve, reject) => {
      const s = document.createElement('script');
      s.src = D3_SRC;
      s.async = true;
      s.onload = () => resolve();
      s.onerror = () => reject(new Error('Failed to load D3'));
      document.head.appendChild(s);
    });
    return d3Loading;
  }

  // Simple palettes (kept deterministic across renders)
  const PALETTE_A = ['#60A5FA', '#34D399', '#F59E0B', '#A78BFA', '#F472B6', '#F87171', '#22D3EE', '#84CC16', '#FB7185'];
  const PALETTE_B = ['#94A3B8', '#A3A3A3', '#9CA3AF', '#C084FC', '#FDA4AF', '#FCA5A5', '#67E8F9', '#A3E635', '#FDBA74'];

  function buildMap(values) {
    const unique = Array.from(new Set(values.filter(Boolean)));
    const map = new Map();
    unique.forEach((v, i) => map.set(v, PALETTE_A[i % PALETTE_A.length]));
    return map;
  }

  function linkColorMap(values) {
    const unique = Array.from(new Set(values.filter(Boolean)));
    const map = new Map();
    unique.forEach((v, i) => map.set(v, PALETTE_B[i % PALETTE_B.length]));
    return map;
  }

  function radiusForDegree(deg) {
    const d = Math.max(0, +deg || 0);
    const r = 6 + Math.sqrt(d) * 3; // gentle growth
    return Math.max(6, Math.min(r, 24));
  }

  function curvedPath(d) {
    const sx = d.source.x, sy = d.source.y, tx = d.target.x, ty = d.target.y;
    const dx = tx - sx, dy = ty - sy;
    const dr = Math.hypot(dx, dy) * 0.7; // curve radius
    const mx = (sx + tx) / 2;
    const my = (sy + ty) / 2;
    // Offset normal to create a consistent arc
    const nx = -dy / (Math.hypot(dx, dy) || 1);
    const ny = dx / (Math.hypot(dx, dy) || 1);
    const cx = mx + nx * 20;
    const cy = my + ny * 20;
    return `M ${sx},${sy} Q ${cx},${cy} ${tx},${ty}`;
  }

  function buildAdjacency(nodes, links) {
    const idToNode = new Map(nodes.map(n => [n.id, n]));
    const neighbors = new Map();
    nodes.forEach(n => neighbors.set(n.id, new Set()));
    links.forEach(l => {
      const s = typeof l.source === 'object' ? l.source.id : l.source;
      const t = typeof l.target === 'object' ? l.target.id : l.target;
      if (neighbors.has(s)) neighbors.get(s).add(t);
      if (neighbors.has(t)) neighbors.get(t).add(s);
    });
    return { idToNode, neighbors };
  }

  function attachOverlay(container, { onSearch, onToggleNames, onToggleEdgeLabels, onCenter }) {
    const overlay = document.createElement('div');
    overlay.className = 'kg-overlay';

    const primaryRow = document.createElement('div');
    primaryRow.className = 'kg-control-row kg-control-row-primary';

    const secondaryRow = document.createElement('div');
    secondaryRow.className = 'kg-control-row kg-control-row-secondary';

    // search box
    const input = document.createElement('input');
    input.type = 'text';
    input.placeholder = 'Search nodes…';
    input.className = 'nb-input kg-search-input';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') onSearch && onSearch(input.value.trim());
    });

    const searchBtn = document.createElement('button');
    searchBtn.className = 'nb-btn btn-xs nb-cta kg-search-btn';
    searchBtn.textContent = 'Go';
    searchBtn.addEventListener('click', () => onSearch && onSearch(input.value.trim()));

    const namesToggle = document.createElement('button');
    namesToggle.className = 'nb-btn btn-xs kg-toggle';
    namesToggle.type = 'button';
    namesToggle.textContent = 'Names';
    namesToggle.addEventListener('click', () => onToggleNames && onToggleNames());

    const labelToggle = document.createElement('button');
    labelToggle.className = 'nb-btn btn-xs kg-toggle';
    labelToggle.type = 'button';
    labelToggle.textContent = 'Labels';
    labelToggle.addEventListener('click', () => onToggleEdgeLabels && onToggleEdgeLabels());

    const centerBtn = document.createElement('button');
    centerBtn.className = 'nb-btn btn-xs';
    centerBtn.textContent = 'Center';
    centerBtn.addEventListener('click', () => onCenter && onCenter());

    primaryRow.appendChild(input);
    primaryRow.appendChild(searchBtn);

    secondaryRow.appendChild(namesToggle);
    secondaryRow.appendChild(labelToggle);
    secondaryRow.appendChild(centerBtn);

    overlay.appendChild(primaryRow);
    overlay.appendChild(secondaryRow);

    container.style.position = 'relative';
    container.appendChild(overlay);

    return { input, overlay, namesToggle, labelToggle };
  }

  function attachLegends(container, typeColor, relColor) {
    const wrap = document.createElement('div');
    wrap.className = 'kg-legend';

    function section(title, items) {
      const sec = document.createElement('div');
      sec.className = 'nb-card kg-legend-card';
      const h = document.createElement('div'); h.className = 'kg-legend-heading'; h.textContent = title; sec.appendChild(h);
      items.forEach(([label, color]) => {
        const row = document.createElement('div'); row.className = 'kg-legend-row';
        const sw = document.createElement('span'); sw.style.background = color; sw.style.width = '12px'; sw.style.height = '12px'; sw.style.border = '2px solid #000';
        const t = document.createElement('span'); t.textContent = label || '—';
        row.appendChild(sw); row.appendChild(t); sec.appendChild(row);
      });
      return sec;
    }

    const typeItems = Array.from(typeColor.entries());
    if (typeItems.length) wrap.appendChild(section('Entity Type', typeItems));
    const relItems = Array.from(relColor.entries());
    if (relItems.length) wrap.appendChild(section('Relationship', relItems));

    container.appendChild(wrap);
    return wrap;
  }

  async function renderKnowledgeGraph(root) {
    const container = (root || document).querySelector('#knowledge-graph');
    if (!container) return;

    await ensureD3().catch(() => {
      const err = document.createElement('div');
      err.className = 'alert alert-error';
      err.textContent = 'Unable to load graph library (D3).';
      container.appendChild(err);
    });
    if (!window.d3) return;

    // Clear previous render
    container.innerHTML = '';

    const width = container.clientWidth || 800;
    const height = container.clientHeight || 600;

    const et = container.dataset.entityType || '';
    const cc = container.dataset.contentCategory || '';
    const qs = new URLSearchParams();
    if (et) qs.set('entity_type', et);
    if (cc) qs.set('content_category', cc);

    const url = '/knowledge/graph.json' + (qs.toString() ? ('?' + qs.toString()) : '');
    let data;
    try {
      const res = await fetch(url, { headers: { 'Accept': 'application/json' } });
      if (!res.ok) throw new Error('Failed to load graph data');
      data = await res.json();
    } catch (_e) {
      const err = document.createElement('div');
      err.className = 'alert alert-error';
      err.textContent = 'Unable to load graph data.';
      container.appendChild(err);
      return;
    }

    // Color maps
    const typeColor = buildMap(data.nodes.map(n => n.entity_type));
    const relColor = linkColorMap(data.links.map(l => l.relationship_type));
    const { neighbors } = buildAdjacency(data.nodes, data.links);

    // Build overlay controls
    let namesVisible = true;
    let edgeLabelsVisible = true;

    const togglePressedState = (button, state) => {
      if (!button) return;
      button.setAttribute('aria-pressed', state ? 'true' : 'false');
      button.classList.toggle('kg-toggle-active', !!state);
    };

    const { input, namesToggle, labelToggle } = attachOverlay(container, {
      onSearch: (q) => focusSearch(q),
      onToggleNames: () => {
        namesVisible = !namesVisible;
        label.style('display', namesVisible ? null : 'none');
        togglePressedState(namesToggle, namesVisible);
      },
      onToggleEdgeLabels: () => {
        edgeLabelsVisible = !edgeLabelsVisible;
        linkLabel.style('display', edgeLabelsVisible ? null : 'none');
        togglePressedState(labelToggle, edgeLabelsVisible);
      },
      onCenter: () => zoomTo(1, [width / 2, height / 2])
    });

    togglePressedState(namesToggle, namesVisible);
    togglePressedState(labelToggle, edgeLabelsVisible);

    // SVG + zoom
    const svg = d3.select(container)
      .append('svg')
      .attr('width', '100%')
      .attr('height', height)
      .attr('viewBox', [0, 0, width, height])
      .attr('style', 'cursor: grab; touch-action: none; background: transparent;')
      .call(d3.zoom().scaleExtent([0.25, 5]).on('zoom', (event) => {
        g.attr('transform', event.transform);
      }));

    const g = svg.append('g');

    // Defs for arrows
    const defs = svg.append('defs');
    const markerFor = (key, color) => {
      const id = `arrow-${key.replace(/[^a-z0-9_-]/gi, '_')}`;
      if (!document.getElementById(id)) {
        defs.append('marker')
          .attr('id', id)
          .attr('viewBox', '0 -5 10 10')
          .attr('refX', 16)
          .attr('refY', 0)
          .attr('markerWidth', 6)
          .attr('markerHeight', 6)
          .attr('orient', 'auto')
          .append('path')
          .attr('d', 'M0,-5L10,0L0,5')
          .attr('fill', color);
      }
      return `url(#${id})`;
    };

    // Forces
    const linkForce = d3.forceLink(data.links)
      .id(d => d.id)
      .distance(l => 70)
      .strength(0.5);

    const simulation = d3.forceSimulation(data.nodes)
      .force('link', linkForce)
      .force('charge', d3.forceManyBody().strength(-220))
      .force('center', d3.forceCenter(width / 2, height / 2))
      .force('collision', d3.forceCollide().radius(d => radiusForDegree(d.degree) + 6))
      .force('y', d3.forceY(height / 2).strength(0.02))
      .force('x', d3.forceX(width / 2).strength(0.02));

    // Links as paths so we can curve + arrow
    const link = g.append('g')
      .attr('fill', 'none')
      .attr('stroke-opacity', 0.7)
      .selectAll('path')
      .data(data.links)
      .join('path')
      .attr('stroke', d => relColor.get(d.relationship_type) || '#CBD5E1')
      .attr('stroke-width', 1.5)
      .attr('marker-end', d => markerFor(d.relationship_type || 'rel', relColor.get(d.relationship_type) || '#CBD5E1'));

    // Optional edge labels (midpoint)
    const linkLabel = g.append('g')
      .selectAll('text')
      .data(data.links)
      .join('text')
      .attr('font-size', 9)
      .attr('fill', '#475569')
      .attr('text-anchor', 'middle')
      .attr('opacity', 0.7)
      .text(d => d.relationship_type || '');

    // Nodes
    const node = g.append('g')
      .attr('stroke', '#fff')
      .attr('stroke-width', 1.5)
      .selectAll('circle')
      .data(data.nodes)
      .join('circle')
      .attr('r', d => radiusForDegree(d.degree))
      .attr('fill', d => typeColor.get(d.entity_type) || '#94A3B8')
      .attr('cursor', 'pointer')
      .on('mouseenter', function (_evt, d) { setHighlight(d); })
      .on('mouseleave', function () { clearHighlight(); })
      .on('click', function (_evt, d) {
        // pin/unpin on click
        if (d.fx == null) { d.fx = d.x; d.fy = d.y; this.setAttribute('data-pinned', 'true'); }
        else { d.fx = null; d.fy = null; this.removeAttribute('data-pinned'); }
      })
      .call(d3.drag()
        .on('start', (event, d) => {
          if (!event.active) simulation.alphaTarget(0.3).restart();
          d.fx = d.x; d.fy = d.y;
        })
        .on('drag', (event, d) => { d.fx = event.x; d.fy = event.y; })
        .on('end', (event, d) => { if (!event.active) simulation.alphaTarget(0); }));

    node.append('title').text(d => `${d.name} • ${d.entity_type} • deg ${d.degree}`);

    // Labels
    const label = g.append('g')
      .selectAll('text')
      .data(data.nodes)
      .join('text')
      .text(d => d.name)
      .attr('font-size', 11)
      .attr('fill', '#111827')
      .attr('stroke', 'white')
      .attr('paint-order', 'stroke')
      .attr('stroke-width', 3)
      .attr('dx', d => radiusForDegree(d.degree) + 6)
      .attr('dy', 4);

    // Legends
    attachLegends(container, typeColor, relColor);

    // Highlight logic
    function setHighlight(n) {
      const ns = neighbors.get(n.id) || new Set();
      node.attr('opacity', d => (d.id === n.id || ns.has(d.id)) ? 1 : 0.15);
      label.attr('opacity', d => (d.id === n.id || ns.has(d.id)) ? 1 : 0.15);
      link
        .attr('stroke-opacity', d => {
          const s = (typeof d.source === 'object') ? d.source.id : d.source;
          const t = (typeof d.target === 'object') ? d.target.id : d.target;
          return (s === n.id || t === n.id || (ns.has(s) && ns.has(t))) ? 0.9 : 0.05;
        })
        .attr('marker-end', d => {
          const c = relColor.get(d.relationship_type) || '#CBD5E1';
          return markerFor(d.relationship_type || 'rel', c);
        });
      linkLabel.attr('opacity', d => {
        const s = (typeof d.source === 'object') ? d.source.id : d.source;
        const t = (typeof d.target === 'object') ? d.target.id : d.target;
        return (s === n.id || t === n.id) ? 0.9 : 0.05;
      });
    }
    function clearHighlight() {
      node.attr('opacity', 1);
      label.attr('opacity', 1);
      link.attr('stroke-opacity', 0.7);
      linkLabel.attr('opacity', 0.7);
    }

    // Search + center helpers
    function centerOnNode(n) {
      const k = 1.5; // zoom factor
      const x = n.x, y = n.y;
      const transform = d3.zoomIdentity.translate(width / 2 - k * x, height / 2 - k * y).scale(k);
      svg.transition().duration(350).call(zoom.transform, transform);
    }
    function focusSearch(query) {
      if (!query) return;
      const q = query.toLowerCase();
      const found = data.nodes.find(n => (n.name || '').toLowerCase().includes(q));
      if (found) { setHighlight(found); centerOnNode(found); }
    }

    // Expose zoom instance
    const zoom = d3.zoom().scaleExtent([0.25, 5]).on('zoom', (event) => g.attr('transform', event.transform));
    svg.call(zoom);
    function zoomTo(k, center) {
      const transform = d3.zoomIdentity.translate(width / 2 - k * center[0], height / 2 - k * center[1]).scale(k);
      svg.transition().duration(250).call(zoom.transform, transform);
    }

    // Tick update
    simulation.on('tick', () => {
      link.attr('d', curvedPath);
      node.attr('cx', d => d.x).attr('cy', d => d.y);
      label.attr('x', d => d.x).attr('y', d => d.y);
      linkLabel.attr('x', d => (d.source.x + d.target.x) / 2).attr('y', d => (d.source.y + d.target.y) / 2);
    });

    // Resize handling
    const ro = new ResizeObserver(() => {
      const w = container.clientWidth || width;
      const h = container.clientHeight || height;
      svg.attr('viewBox', [0, 0, w, h]).attr('height', h);
      simulation.force('center', d3.forceCenter(w / 2, h / 2));
      simulation.alpha(0.3).restart();
    });
    ro.observe(container);
  }

  function tryRender(root) {
    const container = (root || document).querySelector('#knowledge-graph');
    if (container) renderKnowledgeGraph(root);
  }

  // Expose for debugging/manual re-render
  window.renderKnowledgeGraph = () => renderKnowledgeGraph(document);

  // Full page load
  document.addEventListener('DOMContentLoaded', () => tryRender(document));

  // HTMX partial swaps
  document.body.addEventListener('htmx:afterSettle', (evt) => {
    tryRender(evt && evt.target ? evt.target : document);
  });
})();

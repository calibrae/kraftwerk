<script>
  let { values = [], max = null, height = 40, color = "#6366f1", fill = true, suffix = "" } = $props();

  let svgW = 200;
  let svgH = $derived(height);

  let effMax = $derived(() => {
    if (values.length === 0) return 1;
    if (max != null && max > 0) return max;
    return Math.max(...values, 1);
  });

  let pathD = $derived(() => {
    if (values.length === 0) return "";
    const m = effMax();
    const n = values.length;
    const step = svgW / Math.max(n - 1, 1);
    let d = "";
    values.forEach((v, i) => {
      const x = i * step;
      const y = svgH - (v / m) * svgH;
      d += i === 0 ? `M ${x.toFixed(1)} ${y.toFixed(1)}` : ` L ${x.toFixed(1)} ${y.toFixed(1)}`;
    });
    return d;
  });

  let fillD = $derived(() => {
    const p = pathD();
    if (!p) return "";
    const n = values.length;
    const step = svgW / Math.max(n - 1, 1);
    return `${p} L ${((n - 1) * step).toFixed(1)} ${svgH} L 0 ${svgH} Z`;
  });

  let latest = $derived(values.length > 0 ? values[values.length - 1] : 0);
</script>

<div class="sparkline-wrap">
  <svg viewBox="0 0 {svgW} {svgH}" preserveAspectRatio="none" class="sparkline">
    {#if fill && fillD()}
      <path d={fillD()} fill={color} fill-opacity="0.15" />
    {/if}
    {#if pathD()}
      <path d={pathD()} fill="none" stroke={color} stroke-width="1.5" />
    {/if}
  </svg>
  <div class="sparkline-value" style="color: {color}">
    {latest.toFixed(values.length > 0 && latest < 10 ? 1 : 0)}{suffix}
  </div>
</div>

<style>
  .sparkline-wrap {
    position: relative;
    width: 100%;
    display: block;
  }
  .sparkline {
    width: 100%;
    display: block;
  }
  .sparkline-value {
    position: absolute;
    top: 0;
    right: 0;
    font-size: 11px;
    font-family: 'SF Mono', monospace;
    font-weight: 600;
    line-height: 1;
  }
</style>

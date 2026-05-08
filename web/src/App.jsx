import { useState, useEffect, useCallback } from 'react'
import './App.css'

const API = 'http://127.0.0.1:4317'

function fmt(n) {
  if (Math.abs(n) >= 1e9) return (n/1e9).toFixed(1)+'B'
  if (n >= 1e6) return (n/1e6).toFixed(1)+'M'
  if (n >= 1e3) return (n/1e3).toFixed(0)+'K'
  return String(n)
}
const fmtPct = n => (n*100).toFixed(1)+'%'
const fmtUSD = n => '$'+n.toFixed(2)

function animateValue(el, start, end, duration, fmtFn) {
  if (start === end) { el.textContent = fmtFn(end); return }
  const startTime = performance.now()
  function step(ts) {
    const elapsed = ts - startTime
    const progress = Math.min(elapsed / duration, 1)
    const eased = 1 - Math.pow(1 - progress, 3)
    el.textContent = fmtFn(Math.round(start + (end - start) * eased))
    if (progress < 1) requestAnimationFrame(step)
  }
  requestAnimationFrame(step)
}

function StatCard({ label, value, sub, color }) {
  const ref = useCallback(node => {
    if (node && value !== undefined) {
      const fmtFn = label.includes('Cost') ? fmtUSD : label.includes('Rate') ? fmtPct : fmt
      const prev = parseFloat(node.dataset.prev || 0)
      const v = typeof value === 'number' ? value : 0
      if (label.includes('Rate')) animateValue(node, prev, v, 600, fmtPct)
      else animateValue(node, prev, v, 600, fmtFn)
      node.dataset.prev = v
    }
  }, [value, label])

  const displayVal = typeof value === 'number'
    ? (label.includes('Cost') ? fmtUSD(value) : label.includes('Rate') ? fmtPct(value) : fmt(value))
    : (value || '-')

  return (
    <div className="stat-card">
      <div className="stat-label">{label}</div>
      <div className="stat-value" ref={ref} style={color ? { color } : {}}>{displayVal}</div>
      <div className="stat-sub">{sub}</div>
    </div>
  )
}

function Heatmap({ data }) {
  const canvasRef = useCallback(node => {
    if (!node || !data?.length) return
    const canvas = node
    const ctx = canvas.getContext('2d')
    const dpr = window.devicePixelRatio || 1
    const CELL = 12, GAP = 3, LM = 28, TM = 18, RM = 8, BM = 4
    const COLS = 53, ROWS = 7

    const map = {}; let maxV = 0
    data.forEach(d => { map[d.date] = d.input + d.output; if (map[d.date] > maxV) maxV = map[d.date] })
    if (maxV === 0) maxV = 1

    const today = new Date(); today.setHours(0,0,0,0)
    const endDay = new Date(today); endDay.setDate(endDay.getDate() - endDay.getDay())
    const startDay = new Date(endDay); startDay.setDate(startDay.getDate() - COLS * 7 + 1)

    const grid = []; for (let c = 0; c < COLS; c++) { grid[c] = new Array(ROWS).fill(0) }
    const d = new Date(startDay)
    for (let c = 0; c < COLS; c++) {
      for (let r = 0; r < ROWS; r++) {
        grid[c][r] = map[d.toISOString().slice(0,10)] || 0
        d.setDate(d.getDate() + 1)
      }
    }

    const W = LM + COLS * (CELL + GAP) - GAP + RM
    const H = TM + ROWS * (CELL + GAP) - GAP + BM
    canvas.width = W * dpr; canvas.height = H * dpr
    canvas.style.width = W + 'px'; canvas.style.height = H + 'px'
    ctx.scale(dpr, dpr)

    ctx.fillStyle = '#161b22'; ctx.fillRect(0, 0, W, H)
    const months = ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec']
    let lastM = -1; ctx.fillStyle = '#8b949e'; ctx.font = '10px system-ui'
    for (let c = 0; c < COLS; c++) {
      d.setTime(startDay.getTime()); d.setDate(d.getDate() + c * 7)
      const m = d.getMonth()
      if (m !== lastM) { ctx.fillText(months[m], LM + c * (CELL + GAP), TM - 4); lastM = m }
    }
    const days = ['','Mon','','Wed','','Fri','']
    for (let r = 0; r < ROWS; r++) {
      if (days[r]) { ctx.fillStyle = '#8b949e'; ctx.font = '9px system-ui'; ctx.textAlign = 'right'; ctx.fillText(days[r], LM - 4, TM + r * (CELL + GAP) + CELL - 2) }
    }
    function grade(v) {
      if (v === 0) return '#161b22'
      const p = v / maxV
      if (p < 0.25) return '#0e4429'; if (p < 0.5) return '#006d32'; if (p < 0.75) return '#26a641'; return '#39d353'
    }
    for (let c = 0; c < COLS; c++) {
      for (let r = 0; r < ROWS; r++) {
        const x = LM + c * (CELL + GAP), y = TM + r * (CELL + GAP)
        ctx.fillStyle = grade(grid[c][r])
        ctx.beginPath(); ctx.roundRect(x, y, CELL, CELL, 3); ctx.fill()
      }
    }
  }, [data])

  return <canvas ref={canvasRef} id="heatmap" style={{maxWidth:'100%'}} />
}

function App() {
  const [summary, setSummary] = useState(null)
  const [models, setModels] = useState([])
  const [heatmap, setHeatmap] = useState([])
  const [status, setStatus] = useState('Loading…')

  const load = useCallback(async () => {
    try {
      const [sum, mods, hm] = await Promise.all([
        fetch(API+'/api/summary').then(r=>r.json()),
        fetch(API+'/api/models').then(r=>r.json()),
        fetch(API+'/api/heatmap').then(r=>r.json()),
      ])
      setSummary(sum); setModels(mods); setHeatmap(hm)
      setStatus(`${fmt(sum.message_count)} msgs · ${fmtPct(sum.cache_hit_rate)} CHR`)
    } catch(e) { setStatus('Error: '+e.message) }
  }, [])

  useEffect(() => { load() }, [load])

  const doScan = async () => {
    setStatus('Scanning…')
    try {
      const r = await fetch(API+'/api/scan', {method:'POST'})
      const d = await r.json()
      setStatus(`Scanned ${d.total} msgs (${d.inserted} new)`)
      setTimeout(load, 600)
    } catch(e) { setStatus('Scan failed') }
  }

  let inp=0,out=0,cr=0,cw=0,cost=0
  models.forEach(m=>{inp+=m.input;out+=m.output;cr+=m.cache_read;cw+=m.cache_write||0;cost+=m.estimated_cost||0})
  const total=inp+out+cw
  const chr=inp+cr>0?cr/(inp+cr):0

  return (
    <div className="app">
      <header className="header">
        <div className="header-l">
          <h1>UsageMonitor</h1>
          <span className="ver">v0.1.0</span>
        </div>
        <div className="header-r">
          <span className="status">{status}</span>
          <button className="btn btn-p" onClick={doScan}>Scan</button>
          <button className="btn" onClick={load}>Refresh</button>
        </div>
      </header>

      <main className="content">
        <div className="stats-grid">
          <StatCard label="Total Cost" value={cost} sub="estimated" color="#3fb950" />
          <StatCard label="Total Tokens" value={total} sub="input+output+cache write" />
          <StatCard label="Cache Hit Rate" value={chr} sub="cache_read / total" color="#58a6ff" />
          <StatCard label="Messages" value={summary?.message_count || 0} sub={`${summary?.tool_count || 0} tools`} />
          <StatCard label="Input" value={inp} />
          <StatCard label="Output" value={out} />
          <StatCard label="Cache Read" value={cr} sub="tokens saved" color="#3fb950" />
          <StatCard label="Models" value={models.length} sub={`${summary?.tool_count || 0} sources`} />
        </div>

        <div className="row">
          <div className="card">
            <div className="card-title">Contribution Heatmap</div>
            <div style={{overflowX:'auto'}}><Heatmap data={heatmap} /></div>
          </div>

          <div className="card">
            <div className="card-title">Top Models</div>
            <div className="model-list">
              {models.slice(0,10).map(m => (
                <div key={m.model_id} className="model-row">
                  <span className="model-name" title={m.model_id}>{m.model_id}</span>
                  <span className="model-tokens">{fmt(m.input+m.output)}</span>
                  <span className="model-cost">{fmtUSD(m.estimated_cost||0)}</span>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="card" style={{marginTop:12}}>
          <div className="card-title">All Models</div>
          <div className="table-wrap">
            <table>
              <thead><tr><th>Model</th><th className="num">Input</th><th className="num">Output</th><th className="num">Cache</th><th className="num">Sessions</th><th className="num">Cost</th></tr></thead>
              <tbody>
                {models.map(m => <tr key={m.model_id}>
                  <td className="cell-model">{m.model_id}</td>
                  <td className="num">{fmt(m.input)}</td>
                  <td className="num">{fmt(m.output)}</td>
                  <td className="num" style={{color:'#3fb950'}}>{fmt(m.cache_read)}</td>
                  <td className="num">{m.sessions}</td>
                  <td className="num" style={{fontWeight:600}}>{fmtUSD(m.estimated_cost||0)}</td>
                </tr>)}
              </tbody>
            </table>
          </div>
        </div>
      </main>
    </div>
  )
}

export default App

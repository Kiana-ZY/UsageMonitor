import { useState, useEffect, useCallback, useRef } from 'react'
import './App.css'

const API = 'http://127.0.0.1:4317'

function fmt(n){if(Math.abs(n)>=1e9)return(n/1e9).toFixed(1)+'B';if(n>=1e6)return(n/1e6).toFixed(1)+'M';if(n>=1e3)return(n/1e3).toFixed(0)+'K';return String(n)}
const pct=n=>(n*100).toFixed(1)+'%'
const usd=n=>'$'+n.toFixed(2)
const d8=s=>s?.slice(0,8)||''

// ── Animated Counter ──
function AnimatedValue({value,fmtFn}){
  const ref=useRef(null)
  useEffect(()=>{
    const el=ref.current;if(!el)return
    const start=parseFloat(el.dataset.prev||0),end=typeof value==='number'?value:0
    el.dataset.prev=end
    if(start===end){el.textContent=fmtFn(end);return}
    const st=performance.now()
    function step(ts){const p=Math.min((ts-st)/600,1);const e=1-Math.pow(1-p,3);el.textContent=fmtFn(Math.round(start+(end-start)*e));if(p<1)requestAnimationFrame(step)}
    requestAnimationFrame(step)
  },[value,fmtFn])
  return <span ref={ref}>{fmtFn(value)}</span>
}

// ── Period Selector ──
function PeriodBar({period,setPeriod}){
  const opts=[['7d',7],['30d',30],['90d',90],['all',-1]]
  return <div className="period-bar">{opts.map(([l,v])=>(
    <button key={l} className={`chip ${period===v?'on':''}`} onClick={()=>setPeriod(v)}>{l}</button>
  ))}</div>
}

// ── Heatmap ──
function Heatmap({data}){
  const ref=useCallback(node=>{
    if(!node||!data?.length)return
    const c=node,ctx=c.getContext('2d'),dpr=window.devicePixelRatio||1
    const CELL=12,GAP=3,LM=28,TM=18,RM=8,BM=4,COLS=53,ROWS=7
    const map={};let mx=0;data.forEach(d=>{map[d.date]=d.input+d.output;if(map[d.date]>mx)mx=map[d.date]})
    if(!mx)mx=1
    const tdy=new Date();tdy.setHours(0,0,0,0)
    const ed=new Date(tdy);ed.setDate(ed.getDate()-ed.getDay())
    const sd=new Date(ed);sd.setDate(sd.getDate()-COLS*7+1)
    const g=[];for(let i=0;i<COLS;i++){g[i]=new Array(ROWS).fill(0)}
    const d=new Date(sd)
    for(let i=0;i<COLS;i++){for(let r=0;r<ROWS;r++){g[i][r]=map[d.toISOString().slice(0,10)]||0;d.setDate(d.getDate()+1)}}
    const W=LM+COLS*(CELL+GAP)-GAP+RM,H=TM+ROWS*(CELL+GAP)-GAP+BM
    c.width=W*dpr;c.height=H*dpr;c.style.width=W+'px';c.style.height=H+'px'
    ctx.scale(dpr,dpr);ctx.fillStyle='#161b22';ctx.fillRect(0,0,W,H)
    const mn=['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec']
    let lm=-1;ctx.fillStyle='#8b949e';ctx.font='10px system-ui'
    for(let i=0;i<COLS;i++){d.setTime(sd.getTime());d.setDate(d.getDate()+i*7);const m=d.getMonth();if(m!==lm){ctx.fillText(mn[m],LM+i*(CELL+GAP),TM-4);lm=m}}
    const ds=['','Mon','','Wed','','Fri','']
    for(let r=0;r<ROWS;r++){if(ds[r]){ctx.fillStyle='#8b949e';ctx.font='9px system-ui';ctx.textAlign='right';ctx.fillText(ds[r],LM-4,TM+r*(CELL+GAP)+CELL-2)}}
    const gd=v=>{if(!v)return'#161b22';const p=v/mx;return p<.25?'#0e4429':p<.5?'#006d32':p<.75?'#26a641':'#39d353'}
    for(let i=0;i<COLS;i++)for(let r=0;r<ROWS;r++){const x=LM+i*(CELL+GAP),y=TM+r*(CELL+GAP);ctx.fillStyle=gd(g[i][r]);ctx.beginPath();ctx.roundRect(x,y,CELL,CELL,3);ctx.fill()}
    ctx.textAlign='start'
  },[data])
  return <canvas ref={ref} style={{maxWidth:'100%'}}/>
}

// ── Main App ──
export default function App(){
  const [sum,setSum]=useState(null)
  const [models,setModels]=useState([])
  const [sessions,setSessions]=useState([])
  const [hm,setHm]=useState([])
  const [status,setStatus]=useState('Loading…')
  const [period,setPeriod]=useState(30)
  const [tab,setTab]=useState(0)

  const load=useCallback(async()=>{
    try{const[s,m,se,h]=await Promise.all([
      fetch(API+'/api/summary').then(r=>r.json()),
      fetch(API+'/api/models').then(r=>r.json()),
      fetch(API+'/api/sessions').then(r=>r.json()),
      fetch(API+'/api/heatmap').then(r=>r.json()),
    ]);setSum(s);setModels(m);setSessions(se);setHm(h)
      setStatus(`${fmt(s.message_count)} msgs · ${pct(s.cache_hit_rate)} CHR`)}
    catch(e){setStatus('Error: '+e.message)}
  },[])
  useEffect(()=>{load()},[load])

  const scan=async()=>{setStatus('Scanning…');try{const r=await fetch(API+'/api/scan',{method:'POST'});const d=await r.json();setStatus(`Scanned ${d.total} msgs (${d.inserted} new)`);setTimeout(load,600)}catch(e){setStatus('Scan failed')}}

  let inp=0,out=0,cr=0,cw=0,cost=0
  models.forEach(m=>{inp+=m.input;out+=m.output;cr+=m.cache_read;cw+=m.cache_write||0;cost+=m.estimated_cost||0})
  const total=inp+out+cw,chr=inp+cr>0?cr/(inp+cr):0

  return <div className="app">
    <header className="header">
      <div className="hl"><h1>UsageMonitor</h1><span className="ver">v0.1.0</span></div>
      <div className="hr">
        <span className="status">{status}</span>
        <button className="btn btn-p" onClick={scan}>Scan</button>
      </div>
    </header>

    <div className="tabs">
      {['Overview','Models','Sessions'].map((t,i)=>(
        <button key={t} className={`tab ${tab===i?'on':''}`} onClick={()=>setTab(i)}>{t}</button>
      ))}
    </div>

    <main className="content">
      {tab===0&&<>
        <div className="stats">
          {[{l:'Total Cost',v:cost,f:usd,c:'#3fb950'},{l:'Total Tokens',v:total,f:fmt},{l:'Cache Hit',v:chr,f:pct,c:'#58a6ff'},{l:'Messages',v:sum?.message_count||0,f:fmt,s:`${sum?.tool_count||0} tools`},{l:'Input',v:inp,f:fmt},{l:'Output',v:out,f:fmt},{l:'Cache Read',v:cr,f:fmt,c:'#3fb950',s:'saved'},{l:'Models',v:models.length,f:fmt,s:`$${usd(cost)}`}]
          .map(({l,v,f,c,s},i)=>(
            <div key={l} className="stat fade" style={{animationDelay:(i*50)+'ms'}}>
              <div className="sl">{l}</div>
              <div className="sv" style={c?{color:c}:{}}><AnimatedValue value={v} fmtFn={f}/></div>
              <div className="ss">{s||''}</div>
            </div>
          ))}
        </div>

        <div className="row">
          <div className="card">
            <div className="ct">Daily Usage <PeriodBar period={period} setPeriod={setPeriod}/></div>
            <div style={{position:'relative',height:260}}>
              <DailyChart models={models} period={period}/>
            </div>
          </div>
          <div className="card">
            <div className="ct">Top Models</div>
            <div className="mlist">
              {models.slice(0,10).map(m=>(<div key={m.model_id} className="mrow">
                <span className="mn">{m.model_id}</span>
                <span className="mt">{fmt(m.input+m.output)}</span>
                <span className="mc">{usd(m.estimated_cost||0)}</span>
              </div>))}
            </div>
          </div>
        </div>

        <div className="card" style={{marginTop:12}}>
          <div className="ct"><span>Contribution Heatmap</span></div>
          <div style={{overflowX:'auto'}}><Heatmap data={hm}/></div>
        </div>
      </>}

      {tab===1&&<div className="card">
        <div className="ct">All Models</div>
        <div className="twrap"><table>
          <thead><tr><th>Model</th><th className="num">Input</th><th className="num">Output</th><th className="num">Cache</th><th className="num">Sessions</th><th className="num">Requests</th><th className="num">Cost</th></tr></thead>
          <tbody>{models.map(m=><tr key={m.model_id}>
            <td className="cm">{m.model_id}</td>
            <td className="num">{fmt(m.input)}</td><td className="num">{fmt(m.output)}</td>
            <td className="num" style={{color:'#3fb950'}}>{fmt(m.cache_read)}</td>
            <td className="num">{m.sessions}</td><td className="num">{m.requests}</td>
            <td className="num" style={{fontWeight:600}}>{usd(m.estimated_cost||0)}</td>
          </tr>)}</tbody></table></div>
      </div>}

      {tab===2&&<div className="card">
        <div className="ct">Sessions ({sessions.length})</div>
        <div className="twrap"><table>
          <thead><tr><th>Session</th><th>Client</th><th>Model</th><th className="num">Tokens</th><th className="num">Cache</th><th className="num">Msgs</th><th className="num">Cost</th></tr></thead>
          <tbody>{sessions.slice(0,100).map(s=><tr key={s.session_id}>
            <td><code style={{fontSize:11,color:'#58a6ff'}}>{d8(s.session_id)}</code></td>
            <td><span className={`badge b-${s.client}`}>{s.client}</span></td>
            <td className="cm">{s.model_id}</td>
            <td className="num">{fmt(s.input+s.output)}</td><td className="num" style={{color:'#3fb950'}}>{fmt(s.cache_read)}</td>
            <td className="num">{s.messages}</td><td className="num" style={{fontWeight:500}}>{usd(s.estimated_cost||0)}</td>
          </tr>)}</tbody></table></div>
      </div>}
    </main>
  </div>
}

// ── Daily Chart (Chart.js via CDN) ──
function DailyChart({models,period}){
  const ref=useRef(null)
  const chartRef=useRef(null)
  useEffect(()=>{
    if(typeof Chart==='undefined'){const s=document.createElement('script');s.src='https://cdn.jsdelivr.net/npm/chart.js@4.4';s.onload=()=>draw();document.head.appendChild(s)}
    else draw()
    return ()=>{if(chartRef.current)chartRef.current.destroy()}
  },[models,period])

  function draw(){
    if(!ref.current||typeof Chart==='undefined')return
    if(chartRef.current)chartRef.current.destroy()
    const ctx=ref.current.getContext('2d')
    const groups={}
    models.forEach(m=>{
      // in a real app we'd have per-model daily data; for now use totals split by model
      const key=m.model_id
      if(!groups[key])groups[key]=[]
      groups[key].push(m)
    })
    // Use summary data points
    chartRef.current=new Chart(ctx,{
      type:'bar',
      data:{labels:[],datasets:models.slice(0,10).map((m,i)=>({
        label:m.model_id,data:[m.input+m.output+m.cache_read],
        backgroundColor:['#58a6ff','#3fb950','#bc8cff','#d2991d','#f85149','#56d3c4','#ff7b72','#a371f7','#fdba74','#b3cf58'][i],
        borderRadius:2,borderSkipped:false
      }))},
      options:{responsive:true,maintainAspectRatio:false,
        plugins:{legend:{labels:{color:'#8b949e',font:{size:10},usePointStyle:true,pointStyleWidth:8,padding:12}}},
        scales:{x:{ticks:{color:'#484f58'},grid:{color:'#21262d'}},y:{ticks:{color:'#484f58',callback:v=>fmt(v)},grid:{color:'#21262d'}}}
      }
    })
  }

  return <canvas ref={ref}/>
}

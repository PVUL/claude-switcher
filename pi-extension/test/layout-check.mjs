const fg = (_c, s) => s;
const visibleWidth = (s) => [...s].length;
const truncateToWidth = (s, w, ell = "") => {
  const chars = [...s];
  if (chars.length <= w) return s;
  if (w <= 0) return "";
  const e = [...ell];
  const keep = Math.max(0, w - e.length);
  return chars.slice(0, keep).join("") + ell;
};
const BAR_WIDTH = 8;
function formatReset(resetsAt){const ms=Date.parse(resetsAt)-Date.now();if(Number.isNaN(ms))return undefined;const secs=Math.floor(ms/1000);if(secs<=0)return "resetting";const d=Math.floor(secs/86400),h=Math.floor((secs%86400)/3600),m=Math.floor((secs%3600)/60);if(d>0)return `${d}d ${h}h`;if(h>0)return `${h}h ${m}m`;return `${m}m`;}
function utilColor(p){return p>=90?"error":p>=70?"warning":"success";}
function buildAccountTiers(snap){
  const nameOnly=snap.name;
  if(typeof snap.utilization!=="number") return [`${nameOnly} ${fg("dim","usage n/a")}`, nameOnly];
  const pct=Math.round(snap.utilization),color=utilColor(snap.utilization);
  const filled=Math.min(BAR_WIDTH,Math.max(0,Math.round(snap.utilization/100*BAR_WIDTH)));
  const bar=fg(color,"█".repeat(filled))+fg("dim","░".repeat(BAR_WIDTH-filled));
  const pctStr=fg(color,`${pct}%`),reset=snap.resetsAt?formatReset(snap.resetsAt):undefined;
  const noBar=`${nameOnly} ${pctStr}`,noReset=`${nameOnly}  ${bar} ${pctStr}`;
  if(!reset) return [noReset,noBar,nameOnly];
  return [`${noReset} ${fg("dim",reset)}`,noReset,noBar,nameOnly];
}
function row1(pwd,snap,width){
  const tiers=buildAccountTiers(snap),GAP=2,pwdW=visibleWidth(pwd);
  let account=tiers[tiers.length-1];
  for(const t of tiers){if(visibleWidth(t)+GAP+Math.min(pwdW,8)<=width){account=t;break;}}
  const accountW=visibleWidth(account);
  if(accountW>=width) return truncateToWidth(account,width);
  const pwdAvail=width-accountW-GAP;
  const ell=pwdAvail>=4?"...":"";
  const pwdShown=truncateToWidth(pwd,pwdAvail,ell);
  const pad=Math.max(GAP,width-visibleWidth(pwdShown)-accountW);
  return truncateToWidth(pwdShown+" ".repeat(pad)+account,width);
}
const resetsAt=new Date(Date.now()+(3*3600+30*60)*1000+500).toISOString();
const snap={name:"paul-nhost",utilization:13,resetsAt};
let fail=0;
const r=formatReset(resetsAt); if(!/^3h (29|30)m$/.test(r)){fail++;console.log("formatReset FAIL",r);} else console.log("formatReset OK:",r);
const pwd="~/repos/claude-switcher (main)";
for(const w of [120,100,80,60,45,35,25,15,10,5]){
  const line=row1(pwd,snap,w),vw=visibleWidth(line),ok=vw<=w; if(!ok)fail++;
  console.log(`w=${String(w).padStart(3)} vw=${String(vw).padStart(3)} ${ok?"OK  ":"OVER"} | ${line}`);
}
console.log("-- usage n/a --");
for(const w of [80,40,20,10]){const l=row1(pwd,{name:"client"},w),vw=visibleWidth(l),ok=vw<=w;if(!ok)fail++;console.log(`w=${w} vw=${vw} ${ok?"OK":"OVER"} | ${l}`);}
console.log(fail?`\n${fail} FAILURES`:"\nALL OK");process.exit(fail?1:0);

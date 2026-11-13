import json,sys,re
track,style,kind=sys.argv[1:4]; n=int(sys.argv[4]) if len(sys.argv)>4 else 6
rows=json.load(open('results/cite-%s.json'%track))
fm=lambda s: re.sub('[-]', lambda m:{'':'<i>','':'</i>','':'<b>','':'</b>','':'<sc>','':'</sc>','':'<sup>','':'</sup>'}.get(m.group(),'?'), s)
k=0
for r in rows:
    if 'summary' in r or r['style']!=style: continue
    a=r[kind+'_ref']; b=r[kind+'_ours']
    for i,x in enumerate(a):
        y=b[i] if i<len(b) else None
        if x!=y:
            print(r['doc']); print(' REF ',fm(x)); print(' OURS',fm(y) if y else y); k+=1
            if k>=n: sys.exit()

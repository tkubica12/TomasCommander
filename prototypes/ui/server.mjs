import http from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root=fileURLToPath(new URL(".",import.meta.url));
const files=new Set(["index.html","styles.css","app.mjs","model.mjs"]);
const types={".html":"text/html; charset=utf-8",".css":"text/css; charset=utf-8",".mjs":"text/javascript; charset=utf-8"};
const port=Number(process.env.TC_UI_PORT??4177);
const server=http.createServer(async(req,res)=>{
  const name=(req.url??"/").split("?")[0].slice(1)||"index.html";
  if (!["GET","HEAD"].includes(req.method)) {res.writeHead(405);res.end("Method not allowed");return;}
  if (!files.has(name)) {res.writeHead(404);res.end("Not found");return;}
  try {
    const body=await readFile(path.join(root,name));
    res.writeHead(200,{"Content-Type":types[path.extname(name)],"Cache-Control":"no-store",
      "Content-Security-Policy":"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'none'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'self'",
      "X-Content-Type-Options":"nosniff"});
    res.end(req.method==="HEAD"?undefined:body);
  } catch(error) {
    console.error(`Unable to serve ${name}: ${error.message}`);
    res.writeHead(500);res.end("Unable to load prototype asset");
  }
});
server.on("error",error=>{console.error(`UI server failed: ${error.message}`);process.exitCode=1;});
server.listen(port,"127.0.0.1",()=>console.log(`TomasCommander UI lab: http://127.0.0.1:${port}\nMock files only. Ctrl+C stops the server.`));

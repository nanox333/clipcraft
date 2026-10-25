// Original ClipCraft compute compositor. MIT OR Apache-2.0.
struct Params { a:vec4<u32>, b:vec4<u32>, c:vec4<u32>, d:vec4<u32> }
@group(0) @binding(0) var<storage,read> source:array<u32>;
@group(0) @binding(1) var<storage,read> ink:array<u32>;
@group(0) @binding(2) var<storage,read_write> output:array<u32>;
@group(0) @binding(3) var<uniform> p:Params;
fn unpack(v:u32)->vec4<u32> { return vec4<u32>(v&255u,(v>>8u)&255u,(v>>16u)&255u,v>>24u); }
fn pack(v:vec3<f32>)->u32 {
 let c=vec3<u32>(floor(clamp(v,vec3<f32>(0.0),vec3<f32>(255.0))+vec3<f32>(0.5)));
 return c.x|(c.y<<8u)|(c.z<<16u)|0xff000000u;
}
fn effect(c:vec3<f32>,kind:u32)->vec3<f32> {
 let l=dot(c,vec3<f32>(0.299,0.587,0.114));var r=c;
 switch kind {
 case 1u:{r=vec3<f32>(l);}
 case 2u:{r=vec3<f32>(dot(c,vec3<f32>(0.393,0.769,0.189)),dot(c,vec3<f32>(0.349,0.686,0.168)),dot(c,vec3<f32>(0.272,0.534,0.131)));}
 case 3u:{r=c+vec3<f32>(40.0);}
 case 4u:{r=c*0.6;}
 case 5u:{r=(c-vec3<f32>(128.0))*1.35+vec3<f32>(128.0);}
 case 6u:{r=vec3<f32>(l)+(c-vec3<f32>(l))*1.5;}
 case 7u:{r=vec3<f32>(255.0)-c;}
 default:{}
 }
 return floor(clamp(r,vec3<f32>(0.0),vec3<f32>(255.0))+vec3<f32>(0.5));
}
@compute @workgroup_size(8,8)
fn base(@builtin(global_invocation_id) id:vec3<u32>) {
 let outsize=p.a.zw;if any(id.xy>=outsize){return;}
 let fit=p.b.xy;let offset=p.b.zw;var rgb=vec3<f32>(0.0);
 if all(id.xy>=offset) && all(id.xy-offset<fit) {
  let at=id.xy-offset;let bounds=p.c.w;
  let lo=vec2<u32>(source[bounds+at.x*2u],source[bounds+fit.x*2u+at.y*2u]);
  let hi=vec2<u32>(source[bounds+at.x*2u+1u],source[bounds+fit.x*2u+at.y*2u+1u]);
  var sum=vec4<u32>(0u);
  for(var y=lo.y;y<hi.y;y++){for(var x=lo.x;x<hi.x;x++){sum+=unpack(source[y*p.a.x+x]);}}
  let n=(hi.x-lo.x)*(hi.y-lo.y);let c=(sum+vec4<u32>(n/2u))/n;
  // Match the CPU's 8-bit resize/effect rounding, then source-over opaque black.
  rgb=floor(effect(vec3<f32>(c.xyz),p.c.x)*(f32(c.w)/255.0));
 }
 let amount=bitcast<f32>(p.c.z);
 if p.c.y!=0u {let color=select(0.0,255.0,p.c.y==2u);rgb=floor(rgb*(1.0-amount)+vec3<f32>(color*amount)+vec3<f32>(0.5));}
 output[id.y*outsize.x+id.x]=pack(rgb);
}
@compute @workgroup_size(8,8)
fn overlay(@builtin(global_invocation_id) id:vec3<u32>) {
 if any(id.xy>=p.a.zw){return;}
 let xy=vec2<i32>(id.xy)+vec2<i32>(bitcast<i32>(p.b.x),bitcast<i32>(p.b.y));
 if any(xy<vec2<i32>(0)) || any(xy>=vec2<i32>(p.a.xy)){return;}
 let index=u32(xy.y)*p.a.x+u32(xy.x);
 let s=unpack(ink[p.b.z+id.y*p.a.z+id.x]);
 let amount=f32(s.w)/255.0*bitcast<f32>(p.b.w);
 let c=vec3<f32>(unpack(output[index]).xyz)*(1.0-amount)+vec3<f32>(s.xyz)*amount;
 output[index]=pack(c);
}

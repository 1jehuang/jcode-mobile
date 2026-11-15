#!/usr/bin/env python3
"""Compile against actual Android SDK, then run pure host-Java contract tests.

Host fixtures model only Intent/ClipData storage and malformed access. They do
not establish Android lifecycle/device acceptance. Never executes adb.
"""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SDK = Path(os.environ.get("ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", Path.home() / "Android/Sdk")))
JAVA = Path(os.environ.get("JAVA_HOME", Path.home() / "Android/jdk-21")) / "bin"
HELPER = ROOT / "android/app/src/main/java/org/jcode/mobile/IncomingShare.java"
FIXTURES = {
    "android/os/Bundle.java": "package android.os; public class Bundle {}",
    "android/content/Intent.java": r'''
package android.content;
import java.util.HashMap;
public class Intent {
 public static final String ACTION_SEND="android.intent.action.SEND", EXTRA_TEXT="android.intent.extra.TEXT", EXTRA_STREAM="android.intent.extra.STREAM";
 public static final int FLAG_GRANT_READ_URI_PERMISSION=1, FLAG_GRANT_WRITE_URI_PERMISSION=2, FLAG_GRANT_PERSISTABLE_URI_PERMISSION=64, FLAG_GRANT_PREFIX_URI_PERMISSION=128;
 private String action, type; private Object data; private int flags; private ClipData clip;
 public boolean malformed; private final HashMap<String,Object> extras=new HashMap<>();
 public Intent(String a){action=a;} public String getAction(){return action;} public Intent setAction(String a){action=a;return this;}
 public String getType(){return type;} public Intent setType(String t){type=t;return this;}
 public Object getData(){return data;} public Intent setDataAndType(Object d,String t){data=d;type=t;return this;}
 public boolean hasExtra(String k){if(malformed)throw new IllegalArgumentException("bad parcel");return extras.containsKey(k);}
 public CharSequence getCharSequenceExtra(String k){return (CharSequence)extras.get(k);}
 public Intent putExtra(String k,Object v){extras.put(k,v);return this;} public Intent replaceExtras(android.os.Bundle b){extras.clear();malformed=false;return this;}
 public ClipData getClipData(){return clip;} public void setClipData(ClipData c){clip=c;}
 public int getFlags(){return flags;} public Intent setFlags(int f){flags=f;return this;}
}
''',
    "android/content/ClipData.java": r'''
package android.content;
public class ClipData {
 private final Item[] items; public ClipData(Item... i){items=i;}
 public int getItemCount(){return items.length;} public Item getItemAt(int n){return items[n];}
 public static class Item {
  private final CharSequence text; private final Object uri; private final Intent intent; private final String html;
  public Item(CharSequence t,Object u,Intent i,String h){text=t;uri=u;intent=i;html=h;}
  public CharSequence getText(){return text;} public Object getUri(){return uri;} public Intent getIntent(){return intent;} public String getHtmlText(){return html;}
 }
}
''',
    "org/jcode/mobile/IncomingShareTest.java": r'''
package org.jcode.mobile;
import android.content.Intent;
import android.content.ClipData;
import java.net.URLDecoder;
public class IncomingShareTest {
 static int checks;
 static void check(boolean condition,String message){checks++;if(!condition)throw new AssertionError(message);}
 static Intent share(Object text){return new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT,text);}
 static String repeat(String value,int n){StringBuilder s=new StringBuilder();for(int i=0;i<n;i++)s.append(value);return s.toString();}
 static void rejected(Intent i,String why){check(IncomingShare.consume(i)==null,why);check(IncomingShare.consume(i)==null,"replay rejected "+why);check(!i.hasExtra(Intent.EXTRA_TEXT),"cleared "+why);check(i.getClipData()==null&&i.getData()==null&&i.getType()==null,"payload cleared "+why);}
 public static void main(String[] args)throws Exception {
  check(IncomingShare.consume(null)==null,"null");
  Intent view=new Intent("android.intent.action.VIEW").setDataAndType("jcode://session/123",null).putExtra(Intent.EXTRA_TEXT,"untouched");
  check(IncomingShare.consume(view)==null&&view.hasExtra(Intent.EXTRA_TEXT)&&view.getData()!=null,"non-SEND untouched");
  Intent valid=share("control" + (char)31 + "unsafe");
  // This control-containing example must not be accepted.
  rejected(valid,"embedded C0");
  String payload="hello world + &draft=0 # % \n\t emoji ";
  payload=payload.replace("\\n","\n").replace("\\t","\t")+new String(Character.toChars(0x1f4bb));
  Intent once=share(payload); String route=IncomingShare.consume(once);
  check(route!=null&&route.startsWith("jcode://new?text=")&&route.endsWith("&draft=1"),"draft-only route");
  String encoded=route.substring("jcode://new?text=".length(),route.length()-"&draft=1".length());
  check(payload.equals(URLDecoder.decode(encoded,"UTF-8")),"lossless UTF8 injection-safe encoding");
  check(!encoded.contains("&")&&!encoded.contains("#"),"no query injection");
  check(IncomingShare.consume(once)==null&&!once.hasExtra(Intent.EXTRA_TEXT),"exactly once");
  check(!Intent.ACTION_SEND.equals(once.getAction()),"consumed action persists recreation");
  check(IncomingShare.route(repeat("a",4096))!=null,"exact ASCII limit");
  check(IncomingShare.route(repeat("a",4097))==null,"UTF16 bound");
  check(IncomingShare.route(repeat("é",2048))!=null,"exact UTF8 bound");
  check(IncomingShare.route(repeat("é",2049))==null,"UTF8 oversize");
  String[] invalid={""," \n\t\u00a0", "a\u0000b","a\r\nb","a\u007fb","a\u0085b","a\u202eb","a\u2066b","a\u200eb","a\uffffb","\ud800","\udc00","\ud800x"};
  for(String t:invalid)rejected(share(t),"unsafe/empty text");
  rejected(share(null),"missing text"); rejected(share(42),"wrong extra type"); rejected(share(new StringBuilder("styled")),"not plain String");
  rejected(share("text").setType("text/html"),"HTML mime"); rejected(share("text").setType("text/plain; charset=utf-8"),"nonexact mime");
  rejected(share("text").setDataAndType("content://private/file","text/plain"),"data URI");
  rejected(share("text").putExtra(Intent.EXTRA_STREAM,"content://private/file"),"stream");
  for(int grant:new int[]{1,2,64,128}){Intent i=share("text").setFlags(grant);rejected(i,"grant");check(i.getFlags()==0,"grant cleared");}
  Intent malformed=share("text");malformed.malformed=true;rejected(malformed,"malformed parcel");
  Intent textClip=share("plain");textClip.setClipData(new ClipData(new ClipData.Item("plain",null,null,null)));
  check(IncomingShare.consume(textClip)!=null,"redundant plain clip accepted");check(textClip.getClipData()==null,"clip consumed");
  ClipData[] clips={new ClipData(),new ClipData(new ClipData.Item("other",null,null,null)),new ClipData(new ClipData.Item("plain","content://x",null,null)),new ClipData(new ClipData.Item("plain",null,new Intent("nested"),null)),new ClipData(new ClipData.Item("plain",null,null,"<b>plain</b>")),new ClipData(new ClipData.Item("plain",null,null,null),new ClipData.Item("plain",null,null,null))};
  for(ClipData c:clips){Intent i=share("plain");i.setClipData(c);rejected(i,"unsafe clip");}
  System.out.println("IncomingShare pure host contract: "+checks+" assertions PASS (not device acceptance)");
 }
}
''',
}


def main():
    scratch = Path(os.environ.get("JCODE_SCRATCH_DIR", ROOT / "target/android/share-tests"))
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="incoming-share-", dir=scratch) as tmp:
        out = Path(tmp)
        sdk_classes = out / "sdk-classes"
        sdk_classes.mkdir()
        subprocess.run([str(JAVA / "javac"), "--release", "8", "-classpath", str(SDK / "platforms/android-36/android.jar"), "-d", str(sdk_classes), str(HELPER)], check=True)
        paths = []
        for relative, content in FIXTURES.items():
            path = out / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
            paths.append(str(path))
        classes = out / "host-classes"
        classes.mkdir()
        subprocess.run([str(JAVA / "javac"), "--release", "8", "-d", str(classes), str(HELPER), *paths], check=True)
        subprocess.run([str(JAVA / "java"), "-cp", str(classes), "org.jcode.mobile.IncomingShareTest"], check=True)


if __name__ == "__main__":
    main()

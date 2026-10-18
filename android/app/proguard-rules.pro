# proguard-rules.pro — Family Connect (Android)
#
# kotlinx.serialization resolves the polymorphic WebSocket frame hierarchy
# (ClientFrame / ServerFrame sealed interfaces, discriminated by "type")
# through generated companion serializer() lookups. R8 must keep those
# members or a release build throws SerializationException on the first
# inbound frame. Rules are the canonical set from the kotlinx.serialization
# README, scoped to this app's namespace.

-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**

# Keep the generated companion + serializer for every @Serializable class
# in the app (WS frames, REST DTOs).
-if @kotlinx.serialization.Serializable class
me.nettrash.familyconnect.**
{
    static **$* *;
}
-keepnames class <1>$$serializer {
    static <1>$$serializer INSTANCE;
}

-keepclassmembers @kotlinx.serialization.Serializable class me.nettrash.familyconnect.** {
    *** Companion;
    *** INSTANCE;
    kotlinx.serialization.KSerializer serializer(...);
}

# Serializable objects (e.g. ClientFrame.Ping / ServerFrame.Pong) keep
# their INSTANCE + serializer pair.
-keepclasseswithmembers class me.nettrash.familyconnect.** {
    public static ** INSTANCE;
    kotlinx.serialization.KSerializer serializer(...);
}

# The rich blocks' page (#97) calls back into RichHost by NAME —
# `window.FcRichBridge.done(json)` from fc-rich.js — so the bridge's
# JavaScript-facing methods must keep their names through R8. AGP's default
# file carries the same rule for every class; said here as well, because a
# renamed `done` fails silently: every drawing times out and every rich
# block shows its source forever.
-keepclassmembers class me.nettrash.familyconnect.rich.RichHost$Bridge {
    @android.webkit.JavascriptInterface <methods>;
}

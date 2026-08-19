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

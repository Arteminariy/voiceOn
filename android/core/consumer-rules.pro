# JNA: классы и нативные методы вызываются из libjnidispatch.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-dontwarn java.awt.**
# Сгенерированные биндинги UniFFI (структуры JNA, колбэки) — по рефлексии.
-keep class io.github.arteminariy.voiceon.core.** { *; }

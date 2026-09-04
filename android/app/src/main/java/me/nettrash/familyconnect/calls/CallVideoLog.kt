/*
 * CallVideoLog.kt
 * Family Connect (Android)
 *
 * ONE logcat tag for the remote-video handoff, because that handoff is
 * spread over three files on three threads — the composable that owns the
 * SurfaceViewRenderer (main), the state machine that holds it until media
 * exists (the app scope, Dispatchers.Default), and WebRTC's signaling
 * thread that announces the far side's track. A report of "no picture from
 * the other side" is only diagnosable if the whole sequence can be pulled
 * out of one `adb logcat -s FcCallVideo:V` after the fact.
 *
 * What each line must answer: WHEN did the surface attach, WHEN did the
 * track arrive, which of the two was first (that ordering is the whole
 * question on the answer-from-a-locked-phone path, where the Compose tree
 * is built fresh while the call is already connecting), was addSink
 * actually called, and did a frame ever arrive.
 *
 * Rules: once-per-call events only — NEVER per frame. Identities are
 * `System.identityHashCode`, never contents; no SDP, no addresses, no
 * names. Cheap enough to stay in a release build, which is the point: the
 * next occurrence is on someone else's phone.
 */

package me.nettrash.familyconnect.calls

import android.util.Log

object CallVideoLog {

    /** The one tag. `adb logcat -s FcCallVideo:V`. */
    const val TAG = "FcCallVideo"

    /**
     * A stable short id for a sink, renderer or track — enough to tell
     * "the surface that attached" from "the surface that detached" when
     * two of them overlap, which is the failure this logging exists for.
     * Never the object's contents.
     */
    fun id(target: Any?): String =
        if (target == null) "none" else "@%08x".format(System.identityHashCode(target))

    /**
     * One event. The calling THREAD is part of every line: main is the
     * composable, DefaultDispatcher is the call machine, and
     * signaling_thread is libwebrtc announcing the track — and which of
     * them got there first is exactly what a black remote view comes down
     * to.
     */
    fun event(message: String) {
        Log.i(TAG, "$message [${Thread.currentThread().name}]")
    }

    /** An event that should not have happened; same tag, so one grep still finds it. */
    fun warn(message: String) {
        Log.w(TAG, "$message [${Thread.currentThread().name}]")
    }
}

import { useEffect, useMemo, useState, useRef } from 'react';
import { Workflow, scanWorkflow } from '../workflow';
import { createUserMessage } from '../types/message';
import { Message } from '../api';

import { substituteParameters } from '../utils/providerUtils';
import { updateSessionUserWorkflowValues } from '../api';
import { userActionHeaders } from '../utils/userAction';
import { useChatContext } from '../contexts/ChatContext';
import { ChatType } from '../types/chat';
import { toastError, toastSuccess } from '../toasts';

export const useWorkflowManager = (chat: ChatType, workflow?: Workflow | null) => {
  const [isParameterModalOpen, setIsParameterModalOpen] = useState(false);
  const [isWorkflowWarningModalOpen, setIsWorkflowWarningModalOpen] = useState(false);
  const [workflowAccepted, setWorkflowAccepted] = useState(false);
  const [isCreateWorkflowModalOpen, setIsCreateWorkflowModalOpen] = useState(false);
  const [hasSecurityWarnings, setHasSecurityWarnings] = useState(false);
  const [readyForAutoUserPrompt, setReadyForAutoUserPrompt] = useState(false);
  const [workflowError, setWorkflowError] = useState<string | null>(null);
  const workflowParameterValues = chat.workflowParameterValues;

  const chatContext = useChatContext();
  const messages = chat.messages;

  // Get workflow parameters from deeplink if available
  const paramsFromConfig =
    (window.appConfig?.get('workflowParameters') as Record<string, string> | null | undefined) ??
    null;
  const workflowParametersFromConfig = useRef<Record<string, string> | null>(paramsFromConfig);

  const messagesRef = useRef(messages);
  const hasCheckedWorkflowRef = useRef(false);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  const finalWorkflow = chat.workflow;
  const resolvedWorkflow = chat.resolvedWorkflow;

  // Initialize parameters from deeplink when workflow is loaded (from backend/deeplink)
  useEffect(() => {
    if (!chatContext || !finalWorkflow) {
      return;
    }

    // Only initialize if we have params from config and haven't set them yet
    const hasNoParameters =
      !chat.workflowParameterValues ||
      (typeof chat.workflowParameterValues === 'object' &&
        Object.keys(chat.workflowParameterValues).length === 0);

    if (workflowParametersFromConfig.current && hasNoParameters) {
      chatContext.setChat({
        ...chatContext.chat,
        workflowParameterValues: workflowParametersFromConfig.current,
      });
    }
  }, [chatContext, finalWorkflow, chat]);

  useEffect(() => {
    if (!chatContext) return;

    // If we have a workflow from navigation state, always set it and reset acceptance state
    // This ensures that when loading a new workflow, we start fresh
    if (workflow) {
      // Check if this is actually a different workflow (by comparing title and content)
      const currentWorkflow = chatContext.chat.workflow;
      const isNewWorkflow =
        !currentWorkflow ||
        currentWorkflow.title !== workflow.title ||
        currentWorkflow.instructions !== workflow.instructions ||
        currentWorkflow.prompt !== workflow.prompt ||
        JSON.stringify(currentWorkflow.activities) !== JSON.stringify(workflow.activities);

      if (isNewWorkflow) {
        console.log('Setting new workflow config:', workflow.title);
        // Reset workflow acceptance state when loading a new workflow
        setWorkflowAccepted(false);
        setIsParameterModalOpen(false);
        setIsWorkflowWarningModalOpen(false);
        hasCheckedWorkflowRef.current = false; // Reset check flag for new workflow

        // Initialize with parameters from deeplink if available
        const initialParameterValues = workflowParametersFromConfig.current || null;

        chatContext.setChat({
          ...chatContext.chat,
          workflow: workflow,
          workflowParameterValues: initialParameterValues,
          messages: [],
        });
      }
      return;
    }
  }, [chatContext, workflow]);

  useEffect(() => {
    const checkWorkflowAcceptance = async () => {
      // Only check once per workflow load
      if (hasCheckedWorkflowRef.current) {
        return;
      }

      if (finalWorkflow) {
        hasCheckedWorkflowRef.current = true;

        try {
          const hasAccepted = await window.electron.hasAcceptedWorkflowBefore(finalWorkflow);

          if (!hasAccepted) {
            const securityScanResult = await scanWorkflow(finalWorkflow);
            setHasSecurityWarnings(securityScanResult.has_security_warnings);

            setIsWorkflowWarningModalOpen(true);
          } else {
            setWorkflowAccepted(true);
          }
        } catch {
          setHasSecurityWarnings(false);
          setIsWorkflowWarningModalOpen(true);
        }
      } else {
        setWorkflowAccepted(false);
        setIsWorkflowWarningModalOpen(false);
      }
    };

    checkWorkflowAcceptance();
  }, [finalWorkflow, workflow, chat.messages.length]);

  const filteredParameters = useMemo(() => {
    return finalWorkflow?.parameters ?? [];
  }, [finalWorkflow]);

  // Check if template variables are actually used in the workflow content
  const requiresParameters = useMemo(() => {
    return filteredParameters.length > 0;
  }, [filteredParameters]);

  // Check if all required parameters have been filled in
  const hasAllRequiredParameters = useMemo(() => {
    return !requiresParameters || resolvedWorkflow != null;
  }, [requiresParameters, resolvedWorkflow]);

  const hasMessages = messages.length > 0;
  useEffect(() => {
    // Only show parameter modal if:
    // 1. Workflow requires parameters
    // 2. Workflow has been accepted
    // 3. Not all required parameters have been filled in yet
    // 4. Parameter modal is not already open (prevent multiple opens)
    // 5. No messages in chat yet (don't show after conversation has started)
    if (workflowAccepted && !hasAllRequiredParameters && !isParameterModalOpen && !hasMessages) {
      setIsParameterModalOpen(true);
    }
  }, [
    hasAllRequiredParameters,
    workflowAccepted,
    filteredParameters,
    isParameterModalOpen,
    hasMessages,
    chat.sessionId,
    finalWorkflow?.title,
  ]);

  useEffect(() => {
    if (
      !requiresParameters &&
      chatContext &&
      finalWorkflow &&
      chatContext.chat.resolvedWorkflow !== finalWorkflow
    ) {
      chatContext?.setChat({
        ...chatContext.chat,
        resolvedWorkflow: finalWorkflow,
      });
    }
  }, [requiresParameters, finalWorkflow, chatContext]);

  useEffect(() => {
    setReadyForAutoUserPrompt(true);
  }, []);

  const initialPrompt = useMemo(() => {
    if (!finalWorkflow?.prompt || !workflowAccepted || finalWorkflow?.isScheduledExecution) {
      return '';
    }
    return resolvedWorkflow?.prompt ?? finalWorkflow.prompt;
  }, [finalWorkflow, workflowAccepted, resolvedWorkflow]);

  const handleParameterSubmit = async (inputValues: Record<string, string>) => {
    try {
      let response = await updateSessionUserWorkflowValues({
        path: {
          session_id: chat.sessionId,
        },
        body: {
          userWorkflowValues: inputValues,
        },
        // With the user's proof: a private chat refuses this write to a caller
        // without it (issue #56, QA 2026-09-10).
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      let resolvedWorkflow = response.data?.workflow;
      if (chatContext) {
        chatContext.setChat({
          ...chatContext.chat,
          workflowParameterValues: inputValues,
          resolvedWorkflow,
        });
      }
      setIsParameterModalOpen(false);
    } catch (error) {
      let error_message = 'unknown error';
      if (typeof error === 'object' && error !== null && 'message' in error) {
        error_message = error.message as string;
      } else if (typeof error === 'string') {
        error_message = error;
      }
      console.error('Failed to render workflow with parameters:', error);
      toastError({
        title: 'Workflow rendering failed',
        msg: error_message,
      });
    }
  };

  const handleWorkflowAccept = async () => {
    try {
      if (finalWorkflow) {
        await window.electron.recordWorkflowHash(finalWorkflow);
        setWorkflowAccepted(true);
        setIsWorkflowWarningModalOpen(false);
      }
    } catch (error) {
      console.error('Error recording workflow hash:', error);
      setWorkflowAccepted(true);
      setIsWorkflowWarningModalOpen(false);
    }
  };

  const handleWorkflowCancel = () => {
    setIsWorkflowWarningModalOpen(false);
    window.electron.closeWindow();
  };

  const handleAutoExecution = async (
    append: (message: Message) => void,
    isLoading: boolean,
    onAutoExecute?: () => void
  ) => {
    if (
      finalWorkflow?.isScheduledExecution &&
      finalWorkflow?.prompt &&
      (!requiresParameters || workflowParameterValues) &&
      messages.length === 0 &&
      !isLoading &&
      readyForAutoUserPrompt &&
      workflowAccepted
    ) {
      const finalPrompt = workflowParameterValues
        ? substituteParameters(finalWorkflow.prompt, workflowParameterValues)
        : finalWorkflow.prompt;

      const userMessage = await createUserMessage(finalPrompt);
      append(userMessage);
      onAutoExecute?.();
    }
  };

  const handleWorkflowCreated = (workflow: Workflow) => {
    toastSuccess({
      title: 'Workflow created',
      msg: `"${workflow.title}" has been saved and is ready to use.`,
    });
  };

  const workflowId: string | null =
    (window.appConfig.get('workflowId') as string | null | undefined) ?? null;

  return {
    workflow: finalWorkflow,
    workflowId,
    workflowParameterValues,
    filteredParameters,
    initialPrompt,
    isParameterModalOpen,
    setIsParameterModalOpen,
    readyForAutoUserPrompt,
    handleParameterSubmit,
    handleAutoExecution,
    workflowError,
    setWorkflowError,
    isWorkflowWarningModalOpen,
    setIsWorkflowWarningModalOpen,
    workflowAccepted,
    handleWorkflowAccept,
    handleWorkflowCancel,
    hasSecurityWarnings,
    isCreateWorkflowModalOpen,
    setIsCreateWorkflowModalOpen,
    handleWorkflowCreated,
  };
};
